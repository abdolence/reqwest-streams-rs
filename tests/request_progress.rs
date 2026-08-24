#![cfg(feature = "json")]

//! Progress accounting for streamed request bodies.
//!
//! Callback-based, with no tracing subscriber involved, so this can share a binary safely —
//! unlike the tracing tests, which cannot (see `tests/tracing_progress.rs`).

use futures::StreamExt;
use reqwest_streams::{
    ReqwestStreamBody, ReqwestStreamBodyOptions, ReqwestStreamOutcome, ReqwestStreamProgress,
};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Serialize, Clone)]
struct Item {
    id: u32,
}

fn items(count: u32) -> impl futures::Stream<Item = Item> + Send + 'static {
    futures::stream::iter((0..count).map(|id| Item { id }))
}

type Sink = Arc<Mutex<Vec<ReqwestStreamProgress>>>;

fn recording(sink: &Sink) -> ReqwestStreamBodyOptions {
    let seen = sink.clone();
    ReqwestStreamBodyOptions::new().on_progress(move |progress| {
        seen.lock().unwrap().push(*progress);
    })
}

fn terminal(sink: &Sink) -> ReqwestStreamProgress {
    *sink
        .lock()
        .unwrap()
        .iter()
        .rfind(|p| p.outcome != ReqwestStreamOutcome::InProgress)
        .expect("a terminal snapshot must be reported")
}

#[tokio::test]
async fn reports_the_totals_when_the_body_completes() {
    let sink: Sink = Arc::new(Mutex::new(Vec::new()));
    let body = ReqwestStreamBody::with_options(
        http_streams_core::JsonNewLineStreamFormat::new(),
        items(3),
        recording(&sink),
    );

    let mut stream = body.into_stream();
    let mut bytes = 0u64;
    while let Some(chunk) = stream.next().await {
        bytes += chunk.unwrap().len() as u64;
    }
    drop(stream);

    let last = terminal(&sink);
    assert_eq!(last.outcome, ReqwestStreamOutcome::Completed);
    assert_eq!(last.items, 3, "one item per encoded object");
    assert_eq!(last.bytes, bytes, "bytes must match what was produced");
    assert_eq!(last.errors, 0);
}

/// The outbound analogue of a consumer that stopped reading: the transport dropped the body
/// before it ended, which is what happens when a server answers early or a connection dies.
#[tokio::test]
async fn reports_aborted_when_the_body_is_dropped_early() {
    let sink: Sink = Arc::new(Mutex::new(Vec::new()));
    let body = ReqwestStreamBody::with_options(
        http_streams_core::JsonNewLineStreamFormat::new(),
        items(100),
        recording(&sink),
    );

    let mut stream = body.into_stream();
    let _first = stream.next().await.expect("at least one chunk");
    drop(stream);

    assert_eq!(terminal(&sink).outcome, ReqwestStreamOutcome::Aborted);
}

#[tokio::test]
async fn reports_failed_when_the_source_errors() {
    #[derive(Debug)]
    struct Boom;
    impl std::fmt::Display for Boom {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("boom")
        }
    }
    impl std::error::Error for Boom {}

    let sink: Sink = Arc::new(Mutex::new(Vec::new()));
    let errors = Arc::new(AtomicU64::new(0));
    let seen = errors.clone();

    let source = futures::stream::iter(vec![Ok(Item { id: 1 }), Err(Boom)]);
    let body = ReqwestStreamBody::try_with_options(
        http_streams_core::JsonNewLineStreamFormat::new(),
        source,
        recording(&sink).on_error(move |_| {
            seen.fetch_add(1, Ordering::Relaxed);
        }),
    );

    let mut stream = body.into_stream();
    while stream.next().await.is_some() {}
    drop(stream);

    assert_eq!(errors.load(Ordering::Relaxed), 1, "on_error must fire once");
    let last = terminal(&sink);
    assert_eq!(last.outcome, ReqwestStreamOutcome::Failed);
    assert_eq!(last.errors, 1);
}

/// A body that was built and never sent must report nothing at all: a `?` short-circuiting
/// before `send()` is routine, and reporting those would bury the real aborts in noise.
#[tokio::test]
async fn a_body_that_was_never_polled_reports_nothing() {
    let sink: Sink = Arc::new(Mutex::new(Vec::new()));
    let body = ReqwestStreamBody::with_options(
        http_streams_core::JsonNewLineStreamFormat::new(),
        items(3),
        recording(&sink),
    );
    drop(body);

    assert!(
        sink.lock().unwrap().is_empty(),
        "an unsent body must not report"
    );
}
