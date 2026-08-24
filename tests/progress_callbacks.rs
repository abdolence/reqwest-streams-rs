//! Coverage for the accounting callbacks, which are not gated on the `tracing` feature.
//!
//! Deliberately a separate binary from `tracing_progress.rs`, and deliberately one that never
//! installs a subscriber. `tracing` caches each callsite's interest globally, so a callsite
//! first evaluated with no subscriber registered is cached as "never interested" — which is
//! exactly what these tests do, and exactly what would make the neighbouring tracing tests
//! observe nothing. Keeping the two apart is what lets both be correct.
#![cfg(feature = "json")]

use futures::StreamExt;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use reqwest_streams::{
    JsonStreamResponse, ReqwestStreamOptions, ReqwestStreamOutcome, StreamBodyResult,
};

#[cfg(feature = "csv")]
use reqwest_streams::CsvStreamResponse;

/// Builds a real `reqwest::Response` out of exact byte chunks, with no server and no I/O.
fn response_of(chunks: Vec<Result<&'static str, io::Error>>) -> reqwest::Response {
    let body = reqwest::Body::wrap_stream(futures::stream::iter(
        chunks
            .into_iter()
            .map(|chunk| chunk.map(|text| bytes::Bytes::from_static(text.as_bytes()))),
    ));

    reqwest::Response::from(http::Response::new(body))
}

/// The callbacks are the metrics path: they must fire on their own, with no subscriber and
/// regardless of whether the `tracing` feature is even compiled in.
#[tokio::test(flavor = "current_thread")]
async fn callbacks_fire_without_a_subscriber() {
    let errors = Arc::new(AtomicU64::new(0));
    let terminal_items = Arc::new(AtomicU64::new(0));
    let terminal_bytes = Arc::new(AtomicU64::new(0));

    let seen_errors = errors.clone();
    let seen_items = terminal_items.clone();
    let seen_bytes = terminal_bytes.clone();

    // 6 + 11 + 6 = 23 bytes; the middle line fails to decode.
    let stream = response_of(vec![Ok("\"aaa\"\n{ not json\n\"ccc\"\n")])
        .json_nl_stream_with_options::<String>(
            ReqwestStreamOptions::new()
                .max_obj_len(1024)
                .on_error(move |_| {
                    seen_errors.fetch_add(1, Ordering::Relaxed);
                })
                .on_progress(move |progress| {
                    if progress.outcome != ReqwestStreamOutcome::InProgress {
                        seen_items.store(progress.items, Ordering::Relaxed);
                        seen_bytes.store(progress.bytes, Ordering::Relaxed);
                        assert_eq!(progress.outcome, ReqwestStreamOutcome::Failed);
                    }
                }),
        );

    let results: Vec<StreamBodyResult<String>> = stream.collect().await;

    assert_eq!(results.len(), 3, "the stream must not stop at the bad line");
    assert_eq!(errors.load(Ordering::Relaxed), 1);
    assert_eq!(terminal_items.load(Ordering::Relaxed), 2);
    assert_eq!(terminal_bytes.load(Ordering::Relaxed), 23);
}

/// A consumer that stops reading early is ordinary client usage, not an anomaly.
#[tokio::test(flavor = "current_thread")]
async fn a_consumer_that_stops_early_reports_aborted() {
    let outcome = Arc::new(std::sync::Mutex::new(None));
    let seen = outcome.clone();

    let stream = response_of(vec![Ok(r#"["aaa","bbb","ccc"]"#)])
        .json_array_stream_with_options::<String>(ReqwestStreamOptions::new().on_progress(
            move |progress| {
                if progress.outcome != ReqwestStreamOutcome::InProgress {
                    *seen.lock().unwrap() = Some((progress.outcome, progress.items));
                }
            },
        ));

    let mut stream = Box::pin(stream);
    assert_eq!(stream.next().await.unwrap().unwrap(), "aaa");
    drop(stream);

    assert_eq!(
        *outcome.lock().unwrap(),
        Some((ReqwestStreamOutcome::Aborted, 1))
    );
}

/// Building a stream and dropping it unconsumed is routine — a `?` short-circuits, a function
/// returns early — and none of it may be reported as an abort.
#[tokio::test(flavor = "current_thread")]
async fn a_stream_that_was_never_polled_reports_nothing() {
    let reported = Arc::new(AtomicU64::new(0));
    let seen = reported.clone();

    let stream = response_of(vec![Ok(r#"["aaa"]"#)]).json_array_stream_with_options::<String>(
        ReqwestStreamOptions::new().on_progress(move |_| {
            seen.fetch_add(1, Ordering::Relaxed);
        }),
    );

    drop(stream);

    assert_eq!(reported.load(Ordering::Relaxed), 0);
}

/// With nobody listening the pipeline is left untouched; the stream must still behave.
#[tokio::test(flavor = "current_thread")]
async fn passes_through_when_nobody_listens() {
    let stream = response_of(vec![Ok(r#"["aaa","bbb"]"#)]).json_array_stream::<String>(1024);
    let items: Vec<String> = stream.map(|item| item.unwrap()).collect().await;
    assert_eq!(items, vec!["aaa", "bbb"]);
}

/// The existing positional-argument methods must behave exactly as before.
#[tokio::test(flavor = "current_thread")]
async fn the_existing_api_still_enforces_max_obj_len() {
    let stream =
        response_of(vec![Ok(r#"["aaaaaaaaaaaaaaaaaaaaaaaaaaaa"]"#)]).json_array_stream::<String>(8);
    let results: Vec<StreamBodyResult<String>> = stream.collect().await;

    assert!(results.iter().any(|item| item.is_err()));
}

/// Progress is driven from arriving bytes, not only from decoded items: a single item can take
/// a long time to arrive, and reporting nothing until it lands would defeat the point.
#[tokio::test(flavor = "current_thread")]
async fn progress_is_reported_before_any_item_completes() {
    let ticks = Arc::new(AtomicU64::new(0));
    let seen = ticks.clone();

    // Two chunks of an array that never closes, so not one item is ever decoded.
    let stream = response_of(vec![Ok(r#"["aaa"#), Ok(r#"bbb"#)])
        .json_array_stream_with_options::<String>(
            ReqwestStreamOptions::new()
                .max_obj_len(1024)
                .progress_interval(std::time::Duration::ZERO)
                .on_progress(move |progress| {
                    if progress.outcome == ReqwestStreamOutcome::InProgress {
                        assert_eq!(progress.items, 0, "no item can have completed yet");
                        assert!(progress.bytes > 0, "bytes must have been counted");
                        seen.fetch_add(1, Ordering::Relaxed);
                    }
                }),
        );

    let _results: Vec<StreamBodyResult<String>> = stream.collect().await;

    assert!(
        ticks.load(Ordering::Relaxed) > 0,
        "arriving bytes must report progress even with no completed items"
    );
}

/// `.skip(1)` would drop the header frame whether it decoded or not, hiding a header line that
/// blew the length limit and reporting the stream as cleanly completed.
#[cfg(feature = "csv")]
#[tokio::test(flavor = "current_thread")]
async fn a_failing_csv_header_is_not_swallowed() {
    let errors = Arc::new(AtomicU64::new(0));
    let outcome = Arc::new(std::sync::Mutex::new(None));
    let seen_errors = errors.clone();
    let seen_outcome = outcome.clone();

    // The header line is longer than `max_obj_len`, so framing it fails.
    let stream = response_of(vec![Ok("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
real,row
")])
    .csv_stream_with_options::<(String, String)>(
        true,
        b',',
        ReqwestStreamOptions::new()
            .max_obj_len(8)
            .on_error(move |_| {
                seen_errors.fetch_add(1, Ordering::Relaxed);
            })
            .on_progress(move |progress| {
                if progress.outcome != ReqwestStreamOutcome::InProgress {
                    *seen_outcome.lock().unwrap() = Some(progress.outcome);
                }
            }),
    );

    let _results: Vec<StreamBodyResult<(String, String)>> = stream.collect().await;

    assert!(
        errors.load(Ordering::Relaxed) > 0,
        "the header's framing error must be reported, not skipped"
    );
    assert_eq!(*outcome.lock().unwrap(), Some(ReqwestStreamOutcome::Failed));
}
