use reqwest_streams::*;
use serde::{Deserialize, Serialize};
use std::net::TcpListener;

use axum_streams::*;
use futures::prelude::*;
use tower::make::Shared;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MyTestStructure {
    some_test_field: String,
}

fn source_test_stream() -> impl Stream<Item = MyTestStructure> {
    // Simulating a stream with a plain vector
    stream::iter(vec![
        MyTestStructure {
            some_test_field: "TestValue".to_string()
        };
        1000
    ])
}

async fn test_csv_stream() -> impl axum::response::IntoResponse {
    StreamBodyAs::csv(source_test_stream())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Could not bind ephemeral socket");
    let addr = listener.local_addr().unwrap();
    println!("Listening on {}", addr);

    let svc = axum::Router::new().route("/csv", axum::routing::get(test_csv_stream));

    tokio::spawn(async move {
        let server = hyper::server::Server::from_tcp(listener)
            .unwrap()
            .serve(Shared::new(svc));
        server.await.expect("server error");
    });

    println!("Requesting CSV");

    let resp1 = reqwest::get(format!("http://{}/csv", addr))
        .await?
        .csv_stream::<MyTestStructure>(1024, false, b',');

    let items1: Vec<MyTestStructure> = resp1.try_collect().await?;

    println!("{:#?}", items1);

    Ok(())
}
