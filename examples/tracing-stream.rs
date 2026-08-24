use axum::response::IntoResponse;
use axum::routing::*;
use axum::Router;
use futures::{stream, Stream, StreamExt};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use reqwest_streams::{JsonStreamResponse, ReqwestStreamOptions, ReqwestStreamOutcome};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyTestStructure {
    some_test_field: String,
}

/// A stream slow enough that the progress reports have something to report.
fn source_test_stream() -> impl Stream<Item = MyTestStructure> {
    stream::unfold(0usize, |index| async move {
        if index >= 300 {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        let item = MyTestStructure {
            some_test_field: format!("test{index}"),
        };
        Some((item, index + 1))
    })
}

async fn test_json_array_stream() -> impl IntoResponse {
    axum_streams::StreamBodyAs::json_array(source_test_stream())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The stream accounting lives on the `http_streams_core` target, shared with the
    // server-side crate. `reqwest_streams` is named too so that anything this crate logs
    // itself stays visible: filtering on core alone would silently hide it.
    //
    // `=debug` gives a progress line about once a second, `=trace` one per body chunk, and
    // `=off` silences it.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "reqwest_streams=debug,http_streams_core=debug".into()),
        )
        .init();

    let app = Router::new().route("/json-array", get(test_json_array_stream));

    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // The same accounting without tracing: `on_progress` is where you hook up your own
    // metrics. It fires whether or not the `tracing` feature is enabled.
    let streamed_bytes = Arc::new(AtomicU64::new(0));
    let counter = streamed_bytes.clone();

    let stream = reqwest::get(format!("http://{addr}/json-array"))
        .await?
        .json_array_stream_with_options::<MyTestStructure>(
            ReqwestStreamOptions::new()
                .max_obj_len(64 * 1024)
                .progress_interval(Duration::from_millis(500))
                .on_error(|err| tracing::warn!("my own error hook saw: {err}"))
                .on_progress(move |progress| {
                    // Only the final snapshot carries a terminal outcome.
                    if progress.outcome != ReqwestStreamOutcome::InProgress {
                        counter.fetch_add(progress.bytes, Ordering::Relaxed);
                    }
                }),
        );

    let items: Vec<MyTestStructure> = stream.map(|item| item.unwrap()).collect().await;

    println!("Read {} items", items.len());
    println!(
        "on_progress counted {} bytes",
        streamed_bytes.load(Ordering::Relaxed)
    );

    Ok(())
}
