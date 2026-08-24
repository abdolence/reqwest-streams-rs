//! Observing how much of a streamed response actually arrived.
//!
//! A streaming response is consumed lazily, long after the call that created it returned, so
//! nothing at the call site can say how much of it arrived, whether it was truncated by a
//! decode error, or that the consumer walked away after ten items. This module accounts for
//! all of that and reports it two ways: through the [`on_progress`] / [`on_error`] callbacks,
//! which are always available, and through `tracing` when the `tracing` feature is enabled.
//!
//! [`on_progress`]: ReqwestStreamOptions::on_progress
//! [`on_error`]: ReqwestStreamOptions::on_error

use crate::error::StreamBodyError;
use crate::StreamBodyResult;
use bytes::Bytes;
use futures::{Stream, TryStreamExt};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

/// This is the default capacity of the buffer used by `StreamReader`.
pub(crate) const INITIAL_CAPACITY: usize = 8 * 1024;

/// How often progress is reported when it is not otherwise configured.
///
/// Time-based rather than item-based on purpose: the volume of progress reports is then bound
/// by how long the stream runs and not by how much it carries, so even a multi-million item
/// stream cannot flood the logs.
const DEFAULT_PROGRESS_INTERVAL: Duration = Duration::from_secs(1);

/// How a streamed response ended, or that it is still going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReqwestStreamOutcome {
    /// A periodic snapshot: the response is still being read.
    InProgress,
    /// The response body ended and every item was handed over.
    Completed,
    /// The stream ended after at least one error, so items are missing.
    Failed,
    /// The stream was dropped before the body ended, typically because the consumer stopped
    /// reading early.
    Aborted,
}

impl ReqwestStreamOutcome {
    /// The value used for the `outcome` tracing field.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReqwestStreamOutcome::InProgress => "in_progress",
            ReqwestStreamOutcome::Completed => "completed",
            ReqwestStreamOutcome::Failed => "failed",
            ReqwestStreamOutcome::Aborted => "aborted",
        }
    }
}

/// A snapshot of how much of a streamed response has been read so far.
///
/// `items` counts the objects successfully decoded, so an item that failed to decode is not
/// counted. Note that an item is whatever the format produces: for the Arrow format that is a
/// `RecordBatch`, not a row.
///
/// `bytes` counts the body bytes reqwest handed over, which is *after* transfer- and
/// content-decoding. If you enabled reqwest's `gzip` or `brotli` features this is therefore
/// the decompressed size, not the size on the wire.
#[derive(Debug, Clone, Copy)]
pub struct ReqwestStreamProgress {
    pub items: u64,
    pub bytes: u64,
    pub errors: u64,
    pub elapsed: Duration,
    pub outcome: ReqwestStreamOutcome,
}

/// A callback invoked for every error produced while reading a streamed response.
pub type ReqwestStreamErrorHandler = Arc<dyn Fn(&StreamBodyError) + Send + Sync + 'static>;

/// A callback invoked with progress snapshots while reading a streamed response.
pub type ReqwestStreamProgressHandler = Arc<dyn Fn(&ReqwestStreamProgress) + Send + Sync + 'static>;

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

/// Lets [`ProgressStream`] classify items without being generic over the item type.
///
/// A `T` that appeared only in a `where` clause would be an unconstrained type parameter
/// (E0207), and a `PhantomData<T>` would drag `T`'s auto traits into the stream's type — which
/// would break `csv_stream`, whose `T` carries no `Send + 'b` bound even though the method
/// promises them.
pub(crate) trait ProgressItem {
    fn stream_error(&self) -> Option<&StreamBodyError>;
}

impl<T> ProgressItem for StreamBodyResult<T> {
    fn stream_error(&self) -> Option<&StreamBodyError> {
        self.as_ref().err()
    }
}

/// Shared accounting for one streamed response.
///
/// Bytes are counted on the body stream and items on the decoded stream, so the two counters
/// live in different combinators and share this state. The ordering is `Relaxed` throughout:
/// these are counters, not synchronisation.
struct ProgressState {
    items: AtomicU64,
    bytes: AtomicU64,
    errors: AtomicU64,
    last_emit_micros: AtomicU64,
    next_item_step: AtomicU64,
    polled: AtomicBool,
    finalized: AtomicBool,
    start: Instant,
    interval_micros: Option<u64>,
    item_step: Option<u64>,
    on_error: Option<ReqwestStreamErrorHandler>,
    on_progress: Option<ReqwestStreamProgressHandler>,
    #[cfg(feature = "tracing")]
    span: tracing::Span,
}

/// Checked at `ERROR`, the least verbose level the accounting can produce: a failed stream
/// reports there, so gating any higher would mean `RUST_LOG=reqwest_streams=error` silently
/// loses the totals of the very streams it asked about. Every more verbose filter enables
/// `ERROR` too, so this can never suppress wanted output.
#[cfg(feature = "tracing")]
fn tracing_enabled() -> bool {
    tracing::enabled!(target: "reqwest_streams", tracing::Level::ERROR)
}

#[cfg(not(feature = "tracing"))]
fn tracing_enabled() -> bool {
    false
}

impl ProgressState {
    /// Returns `None` when nobody is listening, in which case every accounting call below
    /// short-circuits on a single `Option` check.
    #[cfg_attr(not(feature = "tracing"), allow(unused_variables))]
    fn maybe_new(
        format: &'static str,
        response: &reqwest::Response,
        options: &ReqwestStreamOptions,
    ) -> Option<Arc<Self>> {
        if options.on_progress.is_none() && options.on_error.is_none() && !tracing_enabled() {
            return None;
        }

        // A step of zero would never advance, so treat it as "disabled" rather than looping.
        let item_step = options.progress_items.filter(|step| *step > 0);

        Some(Arc::new(Self {
            items: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            last_emit_micros: AtomicU64::new(0),
            next_item_step: AtomicU64::new(item_step.unwrap_or(u64::MAX)),
            polled: AtomicBool::new(false),
            finalized: AtomicBool::new(false),
            start: Instant::now(),
            interval_micros: options
                .progress_interval
                .map(|interval| interval.as_micros() as u64),
            item_step,
            on_error: options.on_error.clone(),
            on_progress: options.on_progress.clone(),
            #[cfg(feature = "tracing")]
            span: Self::new_span(format, response, options),
        }))
    }

    /// The span covering the whole stream, created here, while the caller's own span is still
    /// the current one, so collectors nest it under their request rather than orphaning it.
    /// The stream itself is polled later, potentially from an entirely different task.
    ///
    /// Every counter is declared up front as an empty field so it can be filled in later with
    /// [`tracing::Span::record`]: collectors that read span attributes (OpenTelemetry and
    /// friends) then see `items`/`bytes`/`outcome` as structured values on a span whose
    /// duration is the streaming duration, instead of having to parse log messages.
    ///
    /// The response is still owned at this point, which is a client-side opportunity the
    /// server side does not have: `status` and `content_length` go on the span, and the
    /// latter is what lets an operator turn `bytes` into a completion percentage. The URL is
    /// deliberately *not* recorded — it carries query strings and userinfo, which routinely
    /// means presigned-URL signatures and `?api_key=`.
    #[cfg(feature = "tracing")]
    fn new_span(
        format: &'static str,
        response: &reqwest::Response,
        options: &ReqwestStreamOptions,
    ) -> tracing::Span {
        let span = tracing::info_span!(
            target: "reqwest_streams",
            "reqwest_streams::response_stream",
            format = format,
            status = response.status().as_u16(),
            // `Option` is a `Value` that simply skips the field when it is empty.
            content_length = response.content_length(),
            max_obj_len = tracing::field::Empty,
            buf_capacity = options.buf_capacity as u64,
            items = tracing::field::Empty,
            bytes = tracing::field::Empty,
            errors = tracing::field::Empty,
            elapsed_ms = tracing::field::Empty,
            outcome = tracing::field::Empty,
        );

        // `usize::MAX` means "no limit", which is noise rather than information.
        if options.max_obj_len != usize::MAX {
            span.record("max_obj_len", options.max_obj_len as u64);
        }

        span
    }

    fn record_bytes(&self, len: u64) {
        let bytes = self.bytes.fetch_add(len, Ordering::Relaxed) + len;
        let items = self.items.load(Ordering::Relaxed);

        #[cfg(feature = "tracing")]
        tracing::trace!(
            target: "reqwest_streams",
            parent: &self.span,
            chunk_bytes = len,
            items,
            bytes,
            "Read an HTTP body chunk"
        );

        // Progress is driven from arriving bytes as well as from decoded items, because a
        // single item can take a long time to arrive: one large Arrow batch, or a JSON array
        // streamed slowly, would otherwise report nothing at all until it completed. Emitting
        // resets the interval, so a chunk and an item cannot both report for the same tick.
        if !self.finalized.load(Ordering::Relaxed) && self.should_emit(items) {
            self.emit(
                ReqwestStreamOutcome::InProgress,
                items,
                bytes,
                self.errors.load(Ordering::Relaxed),
            );
        }
    }

    fn record_item(&self) {
        let items = self.items.fetch_add(1, Ordering::Relaxed) + 1;

        // Nothing may be reported after the summary, or the final snapshot would no longer be
        // final. A consumer is free to keep polling a stream past its end.
        if !self.finalized.load(Ordering::Relaxed) && self.should_emit(items) {
            self.emit(
                ReqwestStreamOutcome::InProgress,
                items,
                self.bytes.load(Ordering::Relaxed),
                self.errors.load(Ordering::Relaxed),
            );
        }
    }

    /// Errors are reported as they happen but are deliberately **not** terminal.
    ///
    /// Only some of them are: `FramedRead` latches its own error state and ends the stream,
    /// but the JSON Lines and CSV formats produce their decoding errors from a successfully
    /// framed line, and the stream carries on to the next one. Finalising here would stop
    /// counting the remaining items of a stream that is still perfectly healthy, so the
    /// terminal outcome is decided at the end instead, from this counter.
    fn record_error(&self, err: &StreamBodyError) {
        self.errors.fetch_add(1, Ordering::Relaxed);

        #[cfg(feature = "tracing")]
        tracing::error!(
            target: "reqwest_streams",
            parent: &self.span,
            error = %err,
            error_kind = err.kind().as_str(),
            "An error occurred while streaming an HTTP body"
        );

        if let Some(handler) = &self.on_error {
            handler(err);
        }
    }

    /// The two triggers are OR'd, and emitting resets both, so an item produces at most one
    /// progress event.
    fn should_emit(&self, items: u64) -> bool {
        let mut emit = false;

        if let Some(step) = self.item_step {
            if items >= self.next_item_step.load(Ordering::Relaxed) {
                // Skip past every step the current count already crossed, so a single poll
                // carrying many items cannot queue up a burst of events.
                self.next_item_step
                    .store(items - (items % step) + step, Ordering::Relaxed);
                emit = true;
            }
        }

        if let Some(interval) = self.interval_micros {
            let elapsed = self.start.elapsed().as_micros() as u64;
            let since_last = elapsed.saturating_sub(self.last_emit_micros.load(Ordering::Relaxed));
            if emit || since_last >= interval {
                self.last_emit_micros.store(elapsed, Ordering::Relaxed);
                emit = true;
            }
        }

        emit
    }

    fn mark_polled(&self) {
        self.polled.store(true, Ordering::Relaxed);
    }

    /// Emits the terminal snapshot, exactly once per stream.
    ///
    /// A stream that was never polled reports nothing at all. Building one and dropping it
    /// unconsumed is routine on the client — a `?` short-circuits, a function returns early —
    /// and reporting those as aborted would bury the real ones in `items=0 bytes=0` noise.
    fn finalize(&self, aborted: bool) {
        if !self.polled.load(Ordering::Relaxed) || self.finalized.swap(true, Ordering::Relaxed) {
            return;
        }

        let items = self.items.load(Ordering::Relaxed);
        let bytes = self.bytes.load(Ordering::Relaxed);
        let errors = self.errors.load(Ordering::Relaxed);

        let outcome = if errors > 0 {
            ReqwestStreamOutcome::Failed
        } else if aborted {
            ReqwestStreamOutcome::Aborted
        } else {
            ReqwestStreamOutcome::Completed
        };

        // Recorded once, here rather than on every progress report: subscribers are free to
        // treat `record` as append-only (`tracing-subscriber`'s formatter does), so writing a
        // field repeatedly makes the rendered span grow with every tick. Once per span also
        // means the values a collector reads are the final ones.
        #[cfg(feature = "tracing")]
        {
            self.span.record("items", items);
            self.span.record("bytes", bytes);
            self.span.record("errors", errors);
            self.span
                .record("elapsed_ms", self.start.elapsed().as_millis() as u64);
            self.span.record("outcome", outcome.as_str());
        }

        self.emit(outcome, items, bytes, errors);
    }

    fn emit(&self, outcome: ReqwestStreamOutcome, items: u64, bytes: u64, errors: u64) {
        let progress = ReqwestStreamProgress {
            items,
            bytes,
            errors,
            elapsed: self.start.elapsed(),
            outcome,
        };

        #[cfg(feature = "tracing")]
        {
            let elapsed_ms = progress.elapsed.as_millis() as u64;

            match outcome {
                // Interim progress is chatter; the summary is the line worth keeping, and a
                // truncated stream is worth an operator's attention.
                ReqwestStreamOutcome::InProgress => tracing::debug!(
                    target: "reqwest_streams",
                    parent: &self.span,
                    items,
                    bytes,
                    elapsed_ms,
                    "Streaming an HTTP body"
                ),
                ReqwestStreamOutcome::Failed => tracing::error!(
                    target: "reqwest_streams",
                    parent: &self.span,
                    items,
                    bytes,
                    errors,
                    elapsed_ms,
                    outcome = outcome.as_str(),
                    "Failed streaming an HTTP body"
                ),
                // Completed, and aborted: a consumer that stops reading early is ordinary.
                _ => tracing::info!(
                    target: "reqwest_streams",
                    parent: &self.span,
                    items,
                    bytes,
                    errors,
                    elapsed_ms,
                    outcome = outcome.as_str(),
                    "Finished streaming an HTTP body"
                ),
            }
        }

        if let Some(handler) = &self.on_progress {
            handler(&progress);
        }
    }
}

/// The accounting handle threaded through one stream's pipeline.
///
/// `None` inside means nobody is listening and every method is a no-op.
#[derive(Clone)]
pub(crate) struct Progress(Option<Arc<ProgressState>>);

impl Progress {
    /// Must be called before `bytes_stream()` consumes the response.
    pub(crate) fn new(
        format: &'static str,
        response: &reqwest::Response,
        options: &ReqwestStreamOptions,
    ) -> Self {
        Progress(ProgressState::maybe_new(format, response, options))
    }
}

/// Counts the bytes of the response body.
///
/// Applied to the byte stream rather than the decoded one so that `bytes` is what actually
/// arrived, independently of how many objects that turned into.
pub(crate) fn count_bytes<'b, S>(
    stream: S,
    progress: &Progress,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'b
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'b,
{
    let progress = progress.clone();
    stream.inspect_ok(move |chunk| {
        if let Some(state) = &progress.0 {
            state.record_bytes(chunk.len() as u64);
        }
    })
}

/// Counts items, reports errors, and owns the outcome state machine.
///
/// It wraps the outermost stream on purpose: every format's errors pass through here, and its
/// `Drop` is the only way to notice a consumer that stopped reading early.
pub(crate) fn instrument<'b, S>(
    stream: S,
    progress: Progress,
) -> impl Stream<Item = S::Item> + Send + 'b
where
    S: Stream + Unpin + Send + 'b,
    S::Item: ProgressItem,
{
    ProgressStream {
        inner: stream,
        progress,
    }
}

struct ProgressStream<S> {
    inner: S,
    progress: Progress,
}

impl<S> Stream for ProgressStream<S>
where
    S: Stream + Unpin,
    S::Item: ProgressItem,
{
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Safe without any projection: `Self: Unpin` whenever `S: Unpin`, which is exactly the
        // bound above. That keeps `#![forbid(unsafe_code)]` intact.
        let this = self.get_mut();

        // Borrowed, not cloned: `progress` and `inner` are disjoint fields, so this avoids an
        // atomic refcount bump on every single poll.
        let Some(state) = this.progress.0.as_ref() else {
            return Pin::new(&mut this.inner).poll_next(cx);
        };

        // Polling here drives the whole pipeline synchronously, reqwest and hyper included, so
        // entering the span gives everything they log the stream's context. `poll_next` is
        // synchronous, so this guard is never held across an await.
        #[cfg(feature = "tracing")]
        let _entered = state.span.enter();

        state.mark_polled();

        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(item)) => {
                match item.stream_error() {
                    Some(err) => state.record_error(err),
                    None => state.record_item(),
                }
                Poll::Ready(Some(item))
            }
            Poll::Ready(None) => {
                state.finalize(false);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> Drop for ProgressStream<S> {
    fn drop(&mut self) {
        if let Some(state) = &self.progress.0 {
            // A no-op when the stream already ran to completion, or was never polled.
            state.finalize(true);
        }
    }
}
