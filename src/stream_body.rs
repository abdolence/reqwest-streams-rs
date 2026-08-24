//! Streaming a request body.
//!
//! The mirror of this crate's response side: instead of decoding a body into a stream of
//! items, this encodes a stream of items into a body you can `POST` or `PUT`.

use crate::observability::{ReqwestStreamErrorHandler, ReqwestStreamProgress, ReqwestStreamProgressHandler};
use crate::{StreamBodyError, StreamBodyResult};
use bytes::Bytes;
use futures::stream::BoxStream;
use futures::{Stream, StreamExt};
use http_streams_core::format::{StreamFormat, StreamFormatEncode};
use http_streams_core::{
    buffer_bytes, buffer_ready_items, count_bytes, count_items, encode_stream, instrument,
    Counting, Direction, Progress, ProgressOptions, Side, StreamContext, StreamErrorKind,
};
use reqwest::header::HeaderValue;
use std::sync::Arc;
use std::time::Duration;

/// Options for a streamed request body.
///
/// Separate from [`ReqwestStreamOptions`] on purpose: that one carries `max_obj_len` and
/// `buf_capacity`, which are decode-side concepts — a guard against a hostile peer, and the
/// size of a read buffer. Neither means anything when you are the one producing the bytes.
///
/// [`ReqwestStreamOptions`]: crate::ReqwestStreamOptions
#[non_exhaustive]
pub struct ReqwestStreamBodyOptions {
    /// Overrides the `Content-Type` the format would otherwise set.
    pub content_type: Option<HeaderValue>,
    /// Coalesce output into chunks of at least this many bytes.
    pub buffering_bytes: Option<usize>,
    /// Coalesce every N items that are ready together into one chunk.
    pub buffering_ready_items: Option<usize>,
    /// Invoked for every error produced while encoding the body.
    pub on_error: Option<ReqwestStreamErrorHandler>,
    /// Invoked for every progress report.
    pub on_progress: Option<ReqwestStreamProgressHandler>,
    /// How often to report interim progress. One second by default.
    pub progress_interval: Option<Duration>,
    /// Additionally report progress every N items.
    pub progress_items: Option<u64>,
}

impl Default for ReqwestStreamBodyOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestStreamBodyOptions {
    /// Default options.
    pub fn new() -> Self {
        Self {
            content_type: None,
            buffering_bytes: None,
            buffering_ready_items: None,
            on_error: None,
            on_progress: None,
            progress_interval: Some(http_streams_core::DEFAULT_PROGRESS_INTERVAL),
            progress_items: None,
        }
    }

    /// Sets the `Content-Type`.
    ///
    /// This is the **only** reliable way to override it, because
    /// [`RequestBuilder::header`] *appends* rather than replaces:
    ///
    /// - Chaining `.header(CONTENT_TYPE, …)` **after** one of the `*_stream_body` methods sends
    ///   two `Content-Type` headers.
    /// - Chaining it **once before** is replaced, which is what you would want.
    /// - Chaining it **twice before** leaves the second value in place, because replacement
    ///   overwrites only the first value for a name. The request then carries two after all.
    ///
    /// [`RequestBuilder::header`]: reqwest::RequestBuilder::header
    pub fn content_type(mut self, content_type: HeaderValue) -> Self {
        self.content_type = Some(content_type);
        self
    }

    /// Coalesce output into chunks of at least `size` bytes.
    ///
    /// Worth setting for formats whose items are small: without it, JSON Lines of short
    /// objects emits one chunked-transfer frame per item.
    ///
    /// Ignored if [`buffering_ready_items`](Self::buffering_ready_items) is also set; the two
    /// are alternatives and the item-based one wins.
    pub fn buffering_bytes(mut self, size: usize) -> Self {
        self.buffering_bytes = Some(size);
        self
    }

    /// Coalesce every `count` items that are ready together into one chunk.
    ///
    /// Takes precedence over [`buffering_bytes`](Self::buffering_bytes) if both are set.
    pub fn buffering_ready_items(mut self, count: usize) -> Self {
        self.buffering_ready_items = Some(count);
        self
    }

    /// Registers a callback invoked for every error produced while encoding the body.
    ///
    /// Worth more here than on the response side. When a request body errors, hyper aborts the
    /// request and [`send`] returns a generic transport error with the original cause usually
    /// flattened away — so this is often the only way to find out *what* failed.
    ///
    /// [`send`]: reqwest::RequestBuilder::send
    pub fn on_error<F>(mut self, handler: F) -> Self
    where
        F: Fn(&StreamBodyError) + Send + Sync + 'static,
    {
        self.on_error = Some(Arc::new(handler));
        self
    }

    /// Registers a callback receiving progress snapshots as the body is uploaded.
    pub fn on_progress<F>(mut self, handler: F) -> Self
    where
        F: Fn(&ReqwestStreamProgress) + Send + Sync + 'static,
    {
        self.on_progress = Some(Arc::new(handler));
        self
    }

    /// Reports progress at most once per `interval`.
    pub fn progress_interval(mut self, interval: Duration) -> Self {
        self.progress_interval = Some(interval);
        self
    }

    /// Additionally reports progress every `items` items.
    pub fn progress_items(mut self, items: u64) -> Self {
        self.progress_items = Some(items);
        self
    }

    fn progress_options(&self) -> ProgressOptions {
        let mut opts = ProgressOptions::new();
        opts.on_error = self.on_error.clone();
        opts.on_progress = self.on_progress.clone();
        opts.progress_interval = self.progress_interval;
        opts.progress_items = self.progress_items;
        opts
    }
}

/// A request body that streams a sequence of items.
///
/// Convert it into a [`reqwest::Body`] with `.into()`, or hand it to
/// [`StreamBodyRequest::stream_body`], which also sets the `Content-Type`.
///
/// # HTTP caveats
///
/// Streaming a *request* body is much less universally supported than streaming a response.
/// None of the following stops it working, but each will surprise you if it is not expected.
///
/// 1. **The body cannot be replayed.** [`RequestBuilder::try_clone`] returns `None` for a
///    streaming body, so retry middleware — `reqwest-retry` and anything like it — cannot
///    retry the request.
/// 2. **A redirect silently sends an empty body.** `reqwest` follows redirects through a
///    middleware that substitutes a default body when the original cannot be cloned, and for
///    `reqwest` that default is an *empty* body. A 307 or 308 on a streaming upload therefore
///    arrives at the new location with nothing in it, and no error is reported. **Use
///    [`redirect::Policy::none`] for streaming uploads** and handle redirects yourself.
/// 3. **Transfer-Encoding is chunked.** No `Content-Length` can be computed, so HTTP/1.1 uses
///    chunked encoding. Some API gateways reject chunked request bodies. HTTP/2 is unaffected.
/// 4. **`Expect: 100-continue` is not supported.** hyper neither sends it nor waits for it, so
///    setting the header by hand does not get you the behaviour: you may upload a great many
///    bytes before learning the request was rejected. When the server *does* answer early, the
///    body is dropped and the outcome is reported as `aborted`.
/// 5. **Buffering reverse proxies defeat streaming.** nginx buffers request bodies by default
///    (`proxy_request_buffering on`), as do many CDNs and API gateways; the server then sees
///    one complete body rather than a stream. Set `proxy_request_buffering off;`.
/// 6. **Timeouts cover the whole exchange.** [`RequestBuilder::timeout`] spans connect through
///    response body, so a slow *source* stream can trip it.
///
/// [`RequestBuilder::try_clone`]: reqwest::RequestBuilder::try_clone
/// [`RequestBuilder::timeout`]: reqwest::RequestBuilder::timeout
/// [`redirect::Policy::none`]: reqwest::redirect::Policy::none
pub struct ReqwestStreamBody {
    stream: BoxStream<'static, StreamBodyResult<Bytes>>,
    content_type: HeaderValue,
}

impl std::fmt::Debug for ReqwestStreamBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReqwestStreamBody")
            .field("content_type", &self.content_type)
            .finish_non_exhaustive()
    }
}

impl ReqwestStreamBody {
    /// A body encoding `stream` with `format`.
    pub fn new<S, T, FMT>(format: FMT, stream: S) -> Self
    where
        FMT: StreamFormatEncode<T> + StreamFormat,
        FMT::Encoder: Send + 'static,
        S: Stream<Item = T> + Send + 'static,
        T: Send + 'static,
    {
        Self::with_options(format, stream, ReqwestStreamBodyOptions::new())
    }

    /// A body encoding a fallible `stream` with `format`.
    ///
    /// Errors from your source stream are forwarded into the body stream, where they abort the
    /// request. Use [`ReqwestStreamBodyOptions::on_error`] to observe them.
    pub fn try_new<S, T, FMT, E>(format: FMT, stream: S) -> Self
    where
        FMT: StreamFormatEncode<T> + StreamFormat,
        FMT::Encoder: Send + 'static,
        S: Stream<Item = Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static,
    {
        Self::try_with_options(format, stream, ReqwestStreamBodyOptions::new())
    }

    /// A body encoding `stream` with `format`, with options.
    pub fn with_options<S, T, FMT>(
        format: FMT,
        stream: S,
        options: ReqwestStreamBodyOptions,
    ) -> Self
    where
        FMT: StreamFormatEncode<T> + StreamFormat,
        FMT::Encoder: Send + 'static,
        S: Stream<Item = T> + Send + 'static,
        T: Send + 'static,
    {
        Self::build(format, stream.map(Ok), options)
    }

    /// A body encoding a fallible `stream` with `format`, with options.
    pub fn try_with_options<S, T, FMT, E>(
        format: FMT,
        stream: S,
        options: ReqwestStreamBodyOptions,
    ) -> Self
    where
        FMT: StreamFormatEncode<T> + StreamFormat,
        FMT::Encoder: Send + 'static,
        S: Stream<Item = Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static,
    {
        let normalised = stream.map(|item| {
            item.map_err(|err| {
                StreamBodyError::new(StreamErrorKind::InputOutputError, Some(err.into()), None)
            })
        });
        Self::build(format, normalised, options)
    }

    fn build<S, T, FMT>(format: FMT, stream: S, options: ReqwestStreamBodyOptions) -> Self
    where
        FMT: StreamFormatEncode<T> + StreamFormat,
        FMT::Encoder: Send + 'static,
        S: Stream<Item = StreamBodyResult<T>> + Send + 'static,
        T: Send + 'static,
    {
        let content_type = options.content_type.clone().unwrap_or_else(|| {
            HeaderValue::from_static(format.default_content_type())
        });

        let context = StreamContext::new(format.format_name(), Direction::Request, Side::Client)
            .content_type(content_type.to_str().unwrap_or_default());
        let context = match options.buffering_bytes {
            Some(bytes) => context.buf_capacity(bytes),
            None => context,
        };
        let progress = Progress::new(&context, &options.progress_options());

        // Items only exist as items upstream of the encoder, so this is where to count them.
        let items = Box::pin(count_items(stream, &progress));
        let bytes = encode_stream(items, format.encoder());

        let buffered: BoxStream<'static, StreamBodyResult<Bytes>> =
            match (options.buffering_ready_items, options.buffering_bytes) {
                (Some(count), _) => Box::pin(buffer_ready_items(bytes, count)),
                (_, Some(size)) => Box::pin(buffer_bytes(bytes, size)),
                (None, None) => Box::pin(bytes),
            };

        let counted = count_bytes(buffered, &progress);

        // Outermost, so its `Drop` coincides with the body's — which is how an upload the
        // server cut short (an early 401 or 413, a dropped connection, a timeout) is noticed.
        // `Counting::Bytes` because by here the items are chunks, counted above as items.
        let stream = Box::pin(instrument(Box::pin(counted), progress, Counting::Bytes));

        Self {
            stream,
            content_type,
        }
    }

    /// The `Content-Type` this body should be sent with.
    ///
    /// [`StreamBodyRequest::stream_body`] and the per-format methods set it for you; this is
    /// for callers building a request by hand.
    pub fn content_type(&self) -> &HeaderValue {
        &self.content_type
    }

    /// The encoded bytes, for callers who are not sending an HTTP request.
    ///
    /// Public on purpose: it makes the body testable without a server, and lets you write the
    /// same encoding to a file, a socket, or an object-store SDK.
    pub fn into_stream(self) -> BoxStream<'static, StreamBodyResult<Bytes>> {
        self.stream
    }
}

impl From<ReqwestStreamBody> for reqwest::Body {
    fn from(body: ReqwestStreamBody) -> Self {
        reqwest::Body::wrap_stream(body.stream)
    }
}

/// Sets a streamed body and its `Content-Type` on a request in one step.
pub trait StreamBodyRequest {
    /// Sets `body` as the request body, along with its `Content-Type`.
    fn stream_body(self, body: ReqwestStreamBody) -> reqwest::RequestBuilder;
}

impl StreamBodyRequest for reqwest::RequestBuilder {
    fn stream_body(self, body: ReqwestStreamBody) -> reqwest::RequestBuilder {
        // `headers`, not `header`: the latter *appends*, so a caller who had already set a
        // Content-Type would end up sending two. `headers` goes through replace semantics.
        let mut headers = reqwest::header::HeaderMap::with_capacity(1);
        headers.insert(reqwest::header::CONTENT_TYPE, body.content_type().clone());
        self.headers(headers).body(reqwest::Body::from(body))
    }
}
