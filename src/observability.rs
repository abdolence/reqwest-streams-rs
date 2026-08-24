//! Observability for streaming responses.
//!
//! The accounting itself lives in [`http_streams_core`] and is shared with `axum-streams`,
//! which had grown an identical implementation independently. What stays here is
//! [`ReqwestStreamOptions`] — it carries decode-side fields that mean nothing on the encode
//! side, and its inherent builder methods could not be added to a type defined elsewhere
//! (E0116) — plus the one thing core cannot see: a [`reqwest::Response`].

use http_streams_core::{Direction, Progress, ProgressOptions, Side, StreamContext};
use std::sync::Arc;
use std::time::Duration;

pub use http_streams_core::{
    StreamErrorHandler as ReqwestStreamErrorHandler, StreamOutcome as ReqwestStreamOutcome,
    StreamProgress as ReqwestStreamProgress, StreamProgressHandler as ReqwestStreamProgressHandler,
};

use crate::StreamBodyError;

/// The default read-buffer size, 8 KiB.
pub(crate) const INITIAL_CAPACITY: usize = http_streams_core::DEFAULT_BUF_CAPACITY;

const DEFAULT_PROGRESS_INTERVAL: Duration = http_streams_core::DEFAULT_PROGRESS_INTERVAL;


/// Options shared by every streaming format.
///
/// Build these with [`ReqwestStreamOptions::new`] and the setters below rather than with a
/// struct literal, so that later options can be added without breaking you.
///
/// # Note on `max_obj_len`
///
/// Unlike the positional-argument methods, which make you choose a limit, a freshly built
/// `ReqwestStreamOptions` does **not** limit object size — [`max_obj_len`] defaults to
/// [`usize::MAX`]. Set it explicitly when reading from a source you do not control.
///
/// [`max_obj_len`]: ReqwestStreamOptions::max_obj_len
#[non_exhaustive]
pub struct ReqwestStreamOptions {
    pub max_obj_len: usize,
    pub buf_capacity: usize,
    pub on_error: Option<ReqwestStreamErrorHandler>,
    pub on_progress: Option<ReqwestStreamProgressHandler>,
    pub progress_interval: Option<Duration>,
    pub progress_items: Option<u64>,
}

impl Default for ReqwestStreamOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestStreamOptions {
    pub fn new() -> Self {
        Self {
            max_obj_len: usize::MAX,
            buf_capacity: INITIAL_CAPACITY,
            on_error: None,
            on_progress: None,
            progress_interval: Some(DEFAULT_PROGRESS_INTERVAL),
            progress_items: None,
        }
    }

    /// The maximum size in bytes of a single decoded object.
    ///
    /// [`usize::MAX`], the default, means no limit.
    pub fn max_obj_len(mut self, max_obj_len: usize) -> Self {
        self.max_obj_len = max_obj_len;
        self
    }

    /// The initial capacity of the stream's decoding buffer.
    pub fn buf_capacity(mut self, buf_capacity: usize) -> Self {
        self.buf_capacity = buf_capacity;
        self
    }

    /// Registers a callback invoked for every error produced while reading the response,
    /// covering both transport errors and decoding errors produced by the format itself.
    ///
    /// The error is still yielded by the stream; this is purely an observation hook. It does
    /// not replace the `tracing` feature: when that feature is enabled both the log event and
    /// this callback fire.
    pub fn on_error<F>(mut self, handler: F) -> Self
    where
        F: Fn(&StreamBodyError) + Send + Sync + 'static,
    {
        self.on_error = Some(Arc::new(handler));
        self
    }

    /// Registers a callback receiving progress snapshots while the response is read: one per
    /// reporting interval or item step, plus a final one carrying the totals and how the
    /// stream ended (completed, failed, or aborted because the consumer stopped reading).
    ///
    /// This is the same accounting the `tracing` feature reports, exposed for metrics: wire it
    /// to a counter and you get streamed items and bytes without depending on tracing at all.
    /// When the feature is enabled both happen.
    ///
    /// The counters are only maintained when someone is listening, so a stream with no
    /// callback and no `tracing` subscriber interested in `reqwest_streams` pays nothing.
    pub fn on_progress<F>(mut self, handler: F) -> Self
    where
        F: Fn(&ReqwestStreamProgress) + Send + Sync + 'static,
    {
        self.on_progress = Some(Arc::new(handler));
        self
    }

    /// Reports progress at most once per `interval` (one second by default).
    ///
    /// Set the field to `None` directly to report on item steps only.
    pub fn progress_interval(mut self, interval: Duration) -> Self {
        self.progress_interval = Some(interval);
        self
    }

    /// Additionally reports progress every `items` items.
    ///
    /// Off by default, and deliberately so: it is a linear step, so a large stream reports a
    /// number of times proportional to its size. Prefer [`Self::progress_interval`] unless you
    /// specifically want item-granular checkpoints.
    pub fn progress_items(mut self, items: u64) -> Self {
        self.progress_items = Some(items);
        self
    }
}

impl ReqwestStreamOptions {
    /// The direction-neutral subset of these options, for the shared accounting.
    pub(crate) fn progress_options(&self) -> ProgressOptions {
        let mut opts = ProgressOptions::new();
        opts.on_error = self.on_error.clone();
        opts.on_progress = self.on_progress.clone();
        opts.progress_interval = self.progress_interval;
        opts.progress_items = self.progress_items;
        opts
    }
}

/// Builds the accounting handle for one response.
///
/// Must be called before `bytes_stream()` consumes the response: `status` and
/// `content_length` go on the span, and the latter is what lets an operator turn `bytes` into
/// a completion percentage. This is a client-side opportunity the server side does not have,
/// and the reason this function lives here rather than in core — core cannot name
/// [`reqwest::Response`].
///
/// The URL is deliberately *not* recorded: it carries query strings and userinfo, which
/// routinely means presigned-URL signatures and `?api_key=`.
pub(crate) fn response_progress(
    format: &'static str,
    response: &reqwest::Response,
    options: &ReqwestStreamOptions,
) -> Progress {
    let mut context = StreamContext::new(format, Direction::Response, Side::Client)
        .status(response.status().as_u16())
        .content_length(response.content_length())
        .buf_capacity(options.buf_capacity);

    // `usize::MAX` means "no limit", which is noise rather than information.
    if options.max_obj_len != usize::MAX {
        context = context.max_obj_len(options.max_obj_len);
    }

    Progress::new(&context, &options.progress_options())
}

/// The shared shape of every decode pipeline in this crate.
///
/// Bytes are counted on the response body, items on the decoded stream, and the outermost
/// wrapper owns error reporting, the outcome, and the `Drop` that notices a consumer which
/// stopped reading early.
pub(crate) fn decode_response<'b, T, FMT>(
    response: reqwest::Response,
    format: FMT,
    format_name: &'static str,
    options: ReqwestStreamOptions,
) -> impl futures::Stream<Item = crate::StreamBodyResult<T>> + Send + 'b
where
    FMT: http_streams_core::format::StreamFormatDecode<T>,
    FMT::Framer: 'b,
    FMT::Parser: 'b,
    FMT::Frame: 'b,
    // Deliberately no `T: 'b`. Formats whose framing is independent of the item type — CSV —
    // keep `T` out of every stored type, so callers are not forced to add an outlives bound to
    // their own public signatures.
{
    // Taken before `bytes_stream()` consumes the response.
    let progress = response_progress(format_name, &response, &options);

    let decode_options = http_streams_core::DecodeOptions::new()
        .max_obj_len(options.max_obj_len)
        .buf_capacity(options.buf_capacity);

    let bytes = http_streams_core::count_bytes(
        futures::TryStreamExt::map_err(response.bytes_stream(), std::io::Error::other),
        &progress,
    );

    let items = http_streams_core::decode_stream(
        bytes,
        format.framer(&decode_options),
        format.parser(),
        &decode_options,
    );

    http_streams_core::instrument(
        Box::pin(items),
        progress,
        http_streams_core::Counting::Items,
    )
}
