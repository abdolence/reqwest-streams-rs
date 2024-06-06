use arrow::array::{Float64Array, Int64Array, RecordBatch, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use reqwest_streams::*;
use std::sync::Arc;

use axum_streams::*;
use futures::prelude::*;

fn source_test_stream(schema: Arc<Schema>) -> impl Stream<Item = RecordBatch> {
    // Simulating a stream with a plain vector and throttling to show how it works
    stream::iter((0i64..10i64).map(move |idx| {
        RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![idx, idx * 2, idx * 3])),
                Arc::new(StringArray::from(vec!["New York", "London", "Gothenburg"])),
                Arc::new(Float64Array::from(vec![40.7128, 51.5074, 57.7089])),
                Arc::new(Float64Array::from(vec![-74.0060, -0.1278, 11.9746])),
            ],
        )
        .unwrap()
    }))
}

async fn test_arrow_ipc_buf() -> impl axum::response::IntoResponse {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("city", DataType::Utf8, false),
        Field::new("lat", DataType::Float64, false),
        Field::new("lng", DataType::Float64, false),
    ]));
    StreamBodyAs::arrow_ipc(schema.clone(), source_test_stream(schema.clone()))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Could not bind ephemeral socket");
    let addr = listener.local_addr().unwrap();
    println!("Listening on {}", addr);

    let svc = axum::Router::new().route("/arrow", axum::routing::get(test_arrow_ipc_buf));

    tokio::spawn(async move {
        let server = axum::serve(listener, svc);
        server.await.expect("server error");
    });

    println!("Requesting JSON");

    let resp1 = reqwest::get(format!("http://{}/arrow", addr))
        .await?
        .arrow_ipc_stream(1024);

    let items1: Vec<RecordBatch> = resp1.try_collect().await?;

    println!("{:#?}", items1);

    Ok(())
}
