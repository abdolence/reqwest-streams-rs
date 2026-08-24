use axum::extract::Request;
use axum::routing::post;
use axum::Router;
use futures::stream;
use reqwest_streams::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyTestStructure {
    some_test_field: String,
}

/// A server that simply reports how much arrived. Receiving the upload *as a stream* needs
/// `axum-streams`' request extractor; this example is about the client side.
async fn ingest(req: Request) -> String {
    let body = axum::body::to_bytes(req.into_body(), usize::MAX)
        .await
        .unwrap();
    format!("received {} bytes", body.len())
}

fn source_test_stream() -> impl stream::Stream<Item = MyTestStructure> {
    stream::iter((0..1000).map(|index| MyTestStructure {
        some_test_field: format!("item-{index}"),
    }))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let app = Router::new().route("/ingest", post(ingest));
        axum::serve(listener, app).await.unwrap();
    });

    // `redirect::Policy::none()` is not incidental. A streaming body cannot be replayed, and a
    // redirect would be followed with an *empty* body and no error at all.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    // The simple path: the format's Content-Type is set for you.
    let response = client
        .post(format!("http://{addr}/ingest"))
        .json_array_stream_body(source_test_stream())
        .send()
        .await?;
    println!("json_array: {}", response.text().await?);

    // The same thing with options: coalesce small items into 8 KiB chunks rather than emitting
    // one chunked-transfer frame per item, and watch the upload progress.
    let body = ReqwestStreamBody::with_options(
        http_streams_core::JsonNewLineStreamFormat::new(),
        source_test_stream(),
        ReqwestStreamBodyOptions::new()
            .buffering_bytes(8 * 1024)
            .on_progress(|progress| {
                println!(
                    "  uploaded {} items / {} bytes ({})",
                    progress.items,
                    progress.bytes,
                    progress.outcome.as_str()
                );
            }),
    );

    let response = client
        .post(format!("http://{addr}/ingest"))
        .stream_body(body)
        .send()
        .await?;
    println!("json_nl: {}", response.text().await?);

    Ok(())
}
