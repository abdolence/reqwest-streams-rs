//! Coverage for what the `tracing` feature actually emits, and for the callbacks that report
//! the same accounting without it.
//!
//! This lives in its own test binary on purpose. `tracing` caches each callsite's interest
//! globally, and a callsite first evaluated while no subscriber is registered is cached as
//! "never interested". Sharing a process with tests that install no subscriber therefore makes
//! a thread-local `set_default` race with that cache; a dedicated binary has no such
//! neighbours.
#![cfg(all(feature = "tracing", feature = "json"))]

use futures::StreamExt;
use std::io;
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

use reqwest_streams::{JsonStreamResponse, ReqwestStreamOptions, StreamBodyResult};

/// Installing a subscriber rebuilds `tracing`'s global callsite interest cache, so two tests
/// doing it at once would race exactly the way this binary exists to avoid. One at a time.
///
/// A `tokio` mutex rather than a `std` one: it is held across awaits, and it does not carry
/// poisoning, so one failing test cannot cascade into the others.
static TRACING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone)]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for SharedWriter {
    type Writer = SharedWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Builds a real `reqwest::Response` out of exact byte chunks, with no server and no I/O.
///
/// A server would make chunk boundaries and timing non-deterministic, and `src/test_client.rs`
/// is `#[cfg(test)]` and so invisible from here anyway.
fn response_of(chunks: Vec<Result<&'static str, io::Error>>) -> reqwest::Response {
    let body = reqwest::Body::wrap_stream(futures::stream::iter(
        chunks
            .into_iter()
            .map(|chunk| chunk.map(|text| bytes::Bytes::from_static(text.as_bytes()))),
    ));

    reqwest::Response::from(http::Response::new(body))
}

/// Runs `body` with a capturing subscriber installed at `level`, and returns what was logged.
async fn capture_at<F, Fut>(level: tracing::Level, body: F) -> String
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let _serialized = TRACING.lock().await;
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(SharedWriter(buffer.clone()))
        .finish();

    // Not `set_global_default`: `capture_at` is called by several tests in this binary.
    let _guard = tracing::subscriber::set_default(subscriber);

    body().await;

    let captured = buffer.lock().unwrap().clone();
    String::from_utf8(captured).unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn reports_the_summary_at_info() {
    let captured = capture_at(tracing::Level::INFO, || async {
        let stream =
            response_of(vec![Ok(r#"["aaa","bbb","ccc"]"#)]).json_array_stream::<String>(1024);
        let items: Vec<String> = stream.map(|item| item.unwrap()).collect().await;
        assert_eq!(items, vec!["aaa", "bbb", "ccc"]);
    })
    .await;

    assert!(captured.contains("INFO"), "unexpected output: {captured}");
    assert!(
        captured.contains("http_streams_core::stream"),
        "the summary must be recorded on the stream span: {captured}"
    );
    assert!(
        captured.contains(r#"format="json_array""#),
        "the span must name the format: {captured}"
    );
    assert!(
        captured.contains("items=3"),
        "unexpected output: {captured}"
    );
    assert!(
        captured.contains("bytes=19"),
        "unexpected output: {captured}"
    );
    assert!(
        captured.contains("errors=0"),
        "unexpected output: {captured}"
    );
    assert!(
        captured.contains(r#"outcome="completed""#),
        "unexpected output: {captured}"
    );
    assert!(
        captured.contains("max_obj_len=1024"),
        "unexpected output: {captured}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn reports_progress_at_debug() {
    // Item-stepped rather than time-based, so the test does not have to sleep for a second.
    let captured = capture_at(tracing::Level::DEBUG, || async {
        let stream = response_of(vec![Ok(r#"["aaa","bbb","ccc"]"#)])
            .json_array_stream_with_options::<String>(
                ReqwestStreamOptions::new()
                    .max_obj_len(1024)
                    .progress_items(1),
            );
        let _items: Vec<StreamBodyResult<String>> = stream.collect().await;
    })
    .await;

    let ticks = captured.matches("Streaming an HTTP body").count();
    assert_eq!(ticks, 3, "expected one tick per item: {captured}");
    assert!(
        captured.contains("Finished streaming an HTTP body"),
        "the summary must still be emitted: {captured}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn reports_per_chunk_at_trace() {
    let captured = capture_at(tracing::Level::TRACE, || async {
        let stream =
            response_of(vec![Ok(r#"["aaa","#), Ok(r#""bbb"]"#)]).json_array_stream::<String>(1024);
        let _items: Vec<StreamBodyResult<String>> = stream.collect().await;
    })
    .await;

    let chunks = captured.matches("Transferred an HTTP body chunk").count();
    assert_eq!(chunks, 2, "expected one event per body chunk: {captured}");
}

#[tokio::test(flavor = "current_thread")]
async fn reports_aborted_when_the_consumer_stops_early() {
    let captured = capture_at(tracing::Level::INFO, || async {
        let stream =
            response_of(vec![Ok(r#"["aaa","bbb","ccc"]"#)]).json_array_stream::<String>(1024);
        let mut stream = Box::pin(stream);
        let first = stream.next().await;
        assert_eq!(first.unwrap().unwrap(), "aaa");
        drop(stream);
    })
    .await;

    assert!(
        captured.contains(r#"outcome="aborted""#),
        "stopping early must report as aborted: {captured}"
    );
    assert!(
        captured.contains("items=1"),
        "the partial total must survive the abort: {captured}"
    );
}

/// Building a stream and dropping it unconsumed is routine on the client — a `?`
/// short-circuits, a function returns early — and every doctest in this crate does exactly
/// that. None of it may report an abort.
#[tokio::test(flavor = "current_thread")]
async fn reports_nothing_when_never_polled() {
    let captured = capture_at(tracing::Level::TRACE, || async {
        let stream = response_of(vec![Ok(r#"["aaa"]"#)]).json_array_stream::<String>(1024);
        drop(stream);
    })
    .await;

    assert!(
        !captured.contains("outcome="),
        "a stream that was never polled must report nothing: {captured}"
    );
}

/// The JSON Lines and CSV formats produce their decoding errors from a successfully framed
/// line, so the stream carries on. Finalizing at the first error would stop counting the rest
/// of a stream that is still perfectly healthy.
#[tokio::test(flavor = "current_thread")]
async fn a_decoding_error_does_not_stop_a_json_nl_stream() {
    let captured = capture_at(tracing::Level::INFO, || async {
        let stream = response_of(vec![Ok("\"aaa\"\n{ not json\n\"ccc\"\n\"ddd\"\n")])
            .json_nl_stream::<String>(1024);
        let results: Vec<StreamBodyResult<String>> = stream.collect().await;

        assert_eq!(results.len(), 4, "the stream must not stop at the bad line");
        assert!(results[0].is_ok());
        assert!(results[1].is_err());
        assert!(results[2].is_ok(), "reading must resume after the error");
        assert!(results[3].is_ok());
    })
    .await;

    assert!(
        captured.contains("items=3"),
        "the items after the bad line must still be counted: {captured}"
    );
    assert!(
        captured.contains("errors=1"),
        "unexpected output: {captured}"
    );
    assert!(
        captured.contains(r#"outcome="failed""#),
        "a stream that saw an error ends as failed: {captured}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn reports_the_error_kind() {
    let captured = capture_at(tracing::Level::ERROR, || async {
        let stream = response_of(vec![Ok(r#"["aaaaaaaaaaaaaaaaaaaaaaaaaaaa"]"#)])
            .json_array_stream::<String>(8);
        let results: Vec<StreamBodyResult<String>> = stream.collect().await;
        assert!(results.iter().any(|item| item.is_err()));
    })
    .await;

    assert!(
        captured.contains(r#"error_kind="max_len""#),
        "the error event must carry its kind: {captured}"
    );
    assert!(
        captured.contains("An error occurred while streaming an HTTP body"),
        "unexpected output: {captured}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn reports_a_transport_error_as_failed() {
    let captured = capture_at(tracing::Level::ERROR, || async {
        let stream = response_of(vec![
            Ok("\"aaa\"\n"),
            Err(io::Error::new(io::ErrorKind::ConnectionReset, "boom")),
        ])
        .json_nl_stream::<String>(1024);
        let _results: Vec<StreamBodyResult<String>> = stream.collect().await;
    })
    .await;

    assert!(
        captured.contains(r#"outcome="failed""#),
        "a broken body must report as failed: {captured}"
    );
}
