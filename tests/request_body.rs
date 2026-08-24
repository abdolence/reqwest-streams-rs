#![cfg(feature = "json")]

//! Tests for streamed request bodies.
//!
//! Two kinds of check, and they prove different things.
//!
//! The byte-parity tests compare this crate's encoder against `axum-streams`. Since 0.29 both
//! sides encode through `http-streams-core`, so this no longer pits two independent
//! implementations against each other: it checks that the two binding layers drive the shared
//! encoder identically, which is what would break if one of them wired up options or framing
//! differently.
//!
//! The cross-crate tests at the bottom are the end-to-end proof, and the reason to depend on a
//! released `axum-streams` rather than a local path: a body this crate uploads is decoded by
//! the extractor a real server would use, over a real socket.

use futures::StreamExt;
use reqwest_streams::{
    JsonStreamResponse, ReqwestStreamBody, ReqwestStreamBodyOptions, StreamBodyResult,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Item {
    id: u32,
    name: String,
}

fn items() -> Vec<Item> {
    vec![
        Item {
            id: 1,
            name: "one".into(),
        },
        Item {
            id: 2,
            name: "two".into(),
        },
    ]
}

async fn collect(body: ReqwestStreamBody) -> Vec<u8> {
    let mut out = Vec::new();
    let mut stream = body.into_stream();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.expect("encoding must not fail"));
    }
    out
}

/// Renders what `axum-streams` produces for the same input.
async fn axum_bytes(body: axum_streams::StreamBodyAs<'static>) -> Vec<u8> {
    use axum::response::IntoResponse;

    let mut stream = body.into_response().into_body().into_data_stream();
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.expect("axum body must render"));
    }
    out
}

#[tokio::test]
async fn json_array_matches_the_server_encoder_byte_for_byte() {
    let ours = collect(ReqwestStreamBody::new(
        http_streams_core::JsonArrayStreamFormat::new(),
        futures::stream::iter(items()),
    ))
    .await;

    let theirs = axum_bytes(axum_streams::StreamBodyAs::json_array(
        futures::stream::iter(items()),
    ))
    .await;

    assert_eq!(
        String::from_utf8(ours).unwrap(),
        String::from_utf8(theirs).unwrap()
    );
}

#[tokio::test]
async fn json_nl_matches_the_server_encoder_byte_for_byte() {
    let ours = collect(ReqwestStreamBody::new(
        http_streams_core::JsonNewLineStreamFormat::new(),
        futures::stream::iter(items()),
    ))
    .await;

    let theirs = axum_bytes(axum_streams::StreamBodyAs::json_nl(futures::stream::iter(
        items(),
    )))
    .await;

    assert_eq!(
        String::from_utf8(ours).unwrap(),
        String::from_utf8(theirs).unwrap()
    );
}

/// A complete encode-then-decode round trip with no server and no I/O: a response can be
/// fabricated from an `http::Response`, so this crate's encoder feeds this crate's decoder.
#[tokio::test]
async fn a_body_round_trips_through_this_crate() {
    let body = ReqwestStreamBody::new(
        http_streams_core::JsonArrayStreamFormat::new(),
        futures::stream::iter(items()),
    );

    let response = reqwest::Response::from(http::Response::new(reqwest::Body::from(body)));
    let decoded: Vec<StreamBodyResult<Item>> =
        response.json_array_stream::<Item>(1024).collect().await;

    let decoded: Vec<Item> = decoded
        .into_iter()
        .map(|r| r.expect("decoding must not fail"))
        .collect();
    assert_eq!(decoded, items());
}

#[tokio::test]
async fn the_content_type_comes_from_the_format() {
    let array = ReqwestStreamBody::new(
        http_streams_core::JsonArrayStreamFormat::new(),
        futures::stream::iter(items()),
    );
    assert_eq!(array.content_type(), "application/json");

    let nl = ReqwestStreamBody::new(
        http_streams_core::JsonNewLineStreamFormat::new(),
        futures::stream::iter(items()),
    );
    assert_eq!(nl.content_type(), "application/jsonstream");
}

#[tokio::test]
async fn the_content_type_can_be_overridden() {
    let body = ReqwestStreamBody::with_options(
        http_streams_core::JsonArrayStreamFormat::new(),
        futures::stream::iter(items()),
        ReqwestStreamBodyOptions::new().content_type(reqwest::header::HeaderValue::from_static(
            "application/x-custom",
        )),
    );
    assert_eq!(body.content_type(), "application/x-custom");
}

/// Buffering must regroup the chunks without changing a single byte.
#[tokio::test]
async fn buffering_does_not_change_the_bytes() {
    let plain = collect(ReqwestStreamBody::new(
        http_streams_core::JsonNewLineStreamFormat::new(),
        futures::stream::iter(items()),
    ))
    .await;

    let buffered = collect(ReqwestStreamBody::with_options(
        http_streams_core::JsonNewLineStreamFormat::new(),
        futures::stream::iter(items()),
        ReqwestStreamBodyOptions::new().buffering_bytes(4),
    ))
    .await;

    assert_eq!(plain, buffered);
}

/// An error from the caller's source stream must reach the body stream rather than being
/// swallowed into a silently truncated upload.
#[tokio::test]
async fn a_source_error_surfaces_in_the_body() {
    #[derive(Debug)]
    struct Boom;
    impl std::fmt::Display for Boom {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("boom")
        }
    }
    impl std::error::Error for Boom {}

    let source = futures::stream::iter(vec![
        Ok(Item {
            id: 1,
            name: "one".into(),
        }),
        Err(Boom),
    ]);

    let body = ReqwestStreamBody::try_new(http_streams_core::JsonArrayStreamFormat::new(), source);

    let chunks: Vec<StreamBodyResult<bytes::Bytes>> = body.into_stream().collect().await;
    assert!(
        chunks.iter().any(|c| c.is_err()),
        "the source error must reach the body stream"
    );

    let encoded: Vec<u8> = chunks
        .into_iter()
        .flatten()
        .flat_map(|b| b.to_vec())
        .collect();
    assert!(
        !String::from_utf8_lossy(&encoded).ends_with(']'),
        "a truncated body must not be closed off as though it were complete"
    );
}

mod wire {
    //! Tests that go over a real socket, because some of what matters here — how many
    //! `Content-Type` headers actually arrive — is invisible in-process.

    use super::*;
    use axum::extract::Request;
    use axum::routing::post;
    use axum::Router;
    use reqwest_streams::JsonStreamRequest;

    /// Echoes back how many `Content-Type` headers arrived, then the body.
    async fn echo(req: Request) -> String {
        let count = req
            .headers()
            .get_all(axum::http::header::CONTENT_TYPE)
            .iter()
            .count();
        let content_type = req
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("<none>")
            .to_string();
        let body = axum::body::to_bytes(req.into_body(), usize::MAX)
            .await
            .expect("body must arrive");
        format!("{count}|{content_type}|{}", String::from_utf8_lossy(&body))
    }

    async fn serve() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route("/ingest", post(echo));
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}/ingest")
    }

    #[tokio::test]
    async fn a_streamed_body_arrives_intact_with_one_content_type() {
        let url = serve().await;

        let response = reqwest::Client::new()
            .post(&url)
            .json_array_stream_body(futures::stream::iter(items()))
            .send()
            .await
            .expect("request must succeed");

        let echoed = response.text().await.unwrap();
        let (count, rest) = echoed.split_once('|').unwrap();
        let (content_type, body) = rest.split_once('|').unwrap();

        assert_eq!(count, "1", "exactly one Content-Type must reach the server");
        assert_eq!(content_type, "application/json");
        assert_eq!(
            body, r#"[{"id":1,"name":"one"},{"id":2,"name":"two"}]"#,
            "the body must arrive intact over chunked transfer-encoding"
        );
    }

    /// The regression behind using `headers()` rather than `header()`: the latter *appends*,
    /// so a caller who had already set a `Content-Type` would send two of them.
    #[tokio::test]
    async fn a_caller_set_content_type_is_replaced_not_duplicated() {
        let url = serve().await;

        let response = reqwest::Client::new()
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "text/plain")
            .json_array_stream_body(futures::stream::iter(items()))
            .send()
            .await
            .expect("request must succeed");

        let echoed = response.text().await.unwrap();
        let (count, rest) = echoed.split_once('|').unwrap();
        let (content_type, _) = rest.split_once('|').unwrap();

        assert_eq!(
            count, "1",
            "the format's Content-Type must replace, not append"
        );
        assert_eq!(content_type, "application/json");
    }
}

/// The pair working together: this crate uploads, `axum-streams` receives.
///
/// Only possible since `axum-streams` 0.29, which added the request-body extractor. This is
/// the check that actually matters for the pair: every other test here exercises one side in
/// isolation, and byte-parity no longer pits two independent implementations against each
/// other now that both encode through the same core.
mod cross_crate {
    use super::*;
    use axum::extract::DefaultBodyLimit;
    use axum::routing::post;
    use axum::Router;

    /// Serves `app` on an ephemeral port and returns the URL of its `/ingest` route.
    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // A streaming upload is exactly what the default body limit exists to stop.
        let app = app.layer(DefaultBodyLimit::disable());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}/ingest")
    }

    mod json {
        use super::*;
        use axum::Json;
        use axum_streams::{JsonArrayStreamFrom, JsonNlStreamFrom};
        use reqwest_streams::JsonStreamRequest;

        async fn ingest_nl(mut items: JsonNlStreamFrom<Item>) -> Json<Vec<Item>> {
            let mut out = Vec::new();
            while let Some(item) = items.next().await {
                out.push(item.expect("no item may fail"));
            }
            Json(out)
        }

        async fn ingest_array(mut items: JsonArrayStreamFrom<Item>) -> Json<Vec<Item>> {
            let mut out = Vec::new();
            while let Some(item) = items.next().await {
                out.push(item.expect("no item may fail"));
            }
            Json(out)
        }

        #[tokio::test]
        async fn json_nl_round_trips() {
            let url = serve(Router::new().route("/ingest", post(ingest_nl))).await;

            let echoed: Vec<Item> = reqwest::Client::new()
                .post(&url)
                .json_nl_stream_body(futures::stream::iter(items()))
                .send()
                .await
                .expect("the upload must succeed")
                .json()
                .await
                .expect("the server must answer with what it decoded");

            assert_eq!(echoed, items());
        }

        #[tokio::test]
        async fn json_array_round_trips() {
            let url = serve(Router::new().route("/ingest", post(ingest_array))).await;

            let echoed: Vec<Item> = reqwest::Client::new()
                .post(&url)
                .json_array_stream_body(futures::stream::iter(items()))
                .send()
                .await
                .expect("the upload must succeed")
                .json()
                .await
                .expect("the server must answer with what it decoded");

            assert_eq!(echoed, items());
        }

        /// Large enough that the body genuinely spans several chunks on the wire rather than
        /// arriving in a single read.
        #[tokio::test]
        async fn a_chunked_upload_survives() {
            let url = serve(Router::new().route("/ingest", post(ingest_nl))).await;

            let many: Vec<Item> = (0..5_000)
                .map(|id| Item {
                    id,
                    name: format!("item-{id}"),
                })
                .collect();

            let echoed: Vec<Item> = reqwest::Client::new()
                .post(&url)
                .json_nl_stream_body(futures::stream::iter(many.clone()))
                .send()
                .await
                .expect("the upload must succeed")
                .json()
                .await
                .expect("the server must answer with what it decoded");

            assert_eq!(echoed, many);
        }
    }

    #[cfg(feature = "csv")]
    mod csv {
        use super::*;
        use axum::Json;
        use axum_streams::CsvStreamFrom;
        use reqwest_streams::CsvStreamRequest;

        async fn ingest(mut rows: CsvStreamFrom<Item>) -> Json<Vec<Item>> {
            let mut out = Vec::new();
            while let Some(row) = rows.next().await {
                out.push(row.expect("no row may fail"));
            }
            Json(out)
        }

        #[tokio::test]
        async fn csv_round_trips() {
            let url = serve(Router::new().route("/ingest", post(ingest))).await;

            let echoed: Vec<Item> = reqwest::Client::new()
                .post(&url)
                .csv_stream_body(futures::stream::iter(items()), true, b',')
                .send()
                .await
                .expect("the upload must succeed")
                .json()
                .await
                .expect("the server must answer with what it decoded");

            assert_eq!(echoed, items());
        }

        /// The framing this pair used to corrupt: a quoted field containing a newline.
        #[tokio::test]
        async fn a_quoted_newline_survives_the_wire() {
            let url = serve(Router::new().route("/ingest", post(ingest))).await;

            let tricky = vec![
                Item {
                    id: 1,
                    name: "line one\nline two".into(),
                },
                Item {
                    id: 2,
                    name: r#"quote" and \ both"#.into(),
                },
            ];

            let echoed: Vec<Item> = reqwest::Client::new()
                .post(&url)
                .csv_stream_body(futures::stream::iter(tricky.clone()), true, b',')
                .send()
                .await
                .expect("the upload must succeed")
                .json()
                .await
                .expect("the server must answer with what it decoded");

            assert_eq!(echoed, tricky);
        }
    }

    #[cfg(feature = "protobuf")]
    mod protobuf {
        use super::*;
        use axum_streams::ProtobufStreamFrom;
        use reqwest_streams::ProtobufStreamRequest;

        #[derive(Clone, PartialEq, prost::Message)]
        pub struct Record {
            #[prost(uint32, tag = "1")]
            pub id: u32,
            #[prost(string, tag = "2")]
            pub name: String,
        }

        fn records() -> Vec<Record> {
            vec![
                Record {
                    id: 1,
                    name: "one".into(),
                },
                // All fields at their default, so this frame is zero bytes long.
                Record::default(),
                Record {
                    id: 3,
                    name: "three".into(),
                },
            ]
        }

        /// Echoes a compact rendering rather than the messages, since prost types are not
        /// serde-serialisable without extra derives.
        async fn ingest(mut items: ProtobufStreamFrom<Record>) -> String {
            let mut parts = Vec::new();
            while let Some(item) = items.next().await {
                let r = item.expect("no message may fail");
                parts.push(format!("{}-{}", r.id, r.name));
            }
            parts.join(",")
        }

        #[tokio::test]
        async fn protobuf_round_trips_including_an_empty_message() {
            let url = serve(Router::new().route("/ingest", post(ingest))).await;

            let echoed = reqwest::Client::new()
                .post(&url)
                .protobuf_stream_body(futures::stream::iter(records()))
                .send()
                .await
                .expect("the upload must succeed")
                .text()
                .await
                .unwrap();

            assert_eq!(echoed, "1-one,0-,3-three");
        }
    }

    #[cfg(feature = "arrow")]
    mod arrow_ipc {
        use super::*;
        use ::arrow::array::{ArrayRef, Int32Array, RecordBatch};
        use ::arrow::datatypes::{DataType, Field, Schema};
        use axum_streams::ArrowIpcStreamFrom;
        use reqwest_streams::ArrowIpcStreamRequest;
        use std::sync::Arc;

        fn schema() -> Arc<Schema> {
            Arc::new(Schema::new(vec![Field::new("v", DataType::Int32, false)]))
        }

        fn batches() -> Vec<RecordBatch> {
            let mk = |vs: Vec<i32>| {
                let col: ArrayRef = Arc::new(Int32Array::from(vs));
                RecordBatch::try_new(schema(), vec![col]).unwrap()
            };
            vec![mk(vec![1, 2, 3]), mk((0..2_000).collect())]
        }

        /// Echoes row counts: a `RecordBatch` is not serde-serialisable.
        async fn ingest(mut batches: ArrowIpcStreamFrom) -> String {
            let mut rows = Vec::new();
            while let Some(batch) = batches.next().await {
                rows.push(batch.expect("no batch may fail").num_rows().to_string());
            }
            rows.join(",")
        }

        #[tokio::test]
        async fn arrow_round_trips() {
            let url = serve(Router::new().route("/ingest", post(ingest))).await;

            let echoed = reqwest::Client::new()
                .post(&url)
                .arrow_ipc_stream_body(schema(), futures::stream::iter(batches()))
                .send()
                .await
                .expect("the upload must succeed")
                .text()
                .await
                .unwrap();

            assert_eq!(echoed, "3,2000");
        }
    }
}
