use reqwest_streams::*;
use serde::{Deserialize, Serialize};

use axum_streams::*;
use futures::prelude::*;
use tokio::net::TcpListener;

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

async fn test_json_array_stream() -> impl axum::response::IntoResponse {
    StreamBodyAs::json_array(source_test_stream())
}

async fn test_json_nl_stream() -> impl axum::response::IntoResponse {
    StreamBodyAs::json_nl(source_test_stream())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Could not bind ephemeral socket");
    let addr = listener.local_addr().unwrap();
    println!("Listening on {}", addr);

    let svc = axum::Router::new()
        .route("/json-array", axum::routing::get(test_json_array_stream))
        .route("/json-nl", axum::routing::get(test_json_nl_stream));

    tokio::spawn(async move {
        let server = axum::serve(listener, svc);
        server.await.expect("server error");
    });

    println!("Requesting JSON");

    let resp1 = reqwest::get(format!("http://{}/json-array", addr))
        .await?
        .json_array_stream::<MyTestStructure>(1024);

    let items1: Vec<MyTestStructure> = resp1.try_collect().await?;

    println!("{:#?}", items1);

    println!("Requesting JSON");
    let resp2 = reqwest::get(format!("http://{}/json-nl", addr))
        .await?
        .json_nl_stream::<MyTestStructure>(1024);

    let items2: Vec<MyTestStructure> = resp2.try_collect().await?;

    println!("{:#?}", items2);

    Ok(())
}
