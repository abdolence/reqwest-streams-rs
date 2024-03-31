use arrow::array::RecordBatch;
use crate::StreamBodyResult;
use async_trait::*;
use futures::stream::BoxStream;
use futures::TryStreamExt;
use crate::arrow_ipc_len_codec::ArrowIpcCodec;

#[async_trait]
pub trait ArrowIpcStreamResponse {
    fn arrow_ipc_stream<'a>(self, max_obj_len: usize) -> BoxStream<'a, StreamBodyResult<RecordBatch>>;
}

#[async_trait]
impl ArrowIpcStreamResponse for reqwest::Response {
    fn arrow_ipc_stream<'a>(self, max_obj_len: usize) -> BoxStream<'a, StreamBodyResult<RecordBatch>> {
        let reader= tokio_util::io::StreamReader::new(
            self.bytes_stream()
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err)),
        );

        let codec = ArrowIpcCodec::new_with_max_length(max_obj_len);
        let frames_reader = tokio_util::codec::FramedRead::new(reader, codec);

        Box::pin(frames_reader.into_stream())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use arrow::array::{Float64Array, Int64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
    use super::*;
    use crate::test_client::*;
    use axum::{routing::*, Router};
    use axum_streams::*;
    use futures::stream;

    fn generate_test_schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("city", DataType::Utf8, false),
            Field::new("lat", DataType::Float64, false),
            Field::new("lng", DataType::Float64, false),
        ]))
    }

    fn generate_test_batches() -> Vec<RecordBatch> {
        (0i64..100i64).map(move |idx| {
            RecordBatch::try_new(
                generate_test_schema(),
                vec![
                    Arc::new(Int64Array::from(vec![idx, idx * 2, idx * 3])),
                    Arc::new(StringArray::from(vec!["New York", "London", "Gothenburg"])),
                    Arc::new(Float64Array::from(vec![40.7128, 51.5074, 57.7089])),
                    Arc::new(Float64Array::from(vec![-74.0060, -0.1278, 11.9746])),
                ],
            )
                .unwrap()
        }).collect()
    }

    #[tokio::test]
    async fn deserialize_proto_stream() {
        let test_stream_vec = generate_test_batches();

        let test_schema = generate_test_schema();
        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route("/", get(|| async { StreamBodyAs::arrow_ipc(test_schema, test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .arrow_ipc_stream(1024);

        let items: Vec<RecordBatch> = res.try_collect().await.unwrap();

        assert_eq!(items, test_stream_vec);
    }

    #[tokio::test]
    async fn deserialize_proto_stream_check_max_len() {
        let test_stream_vec = generate_test_batches();

        let test_schema = generate_test_schema();
        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route("/", get(|| async { StreamBodyAs::arrow_ipc(test_schema, test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .arrow_ipc_stream(10);
        res.try_collect::<Vec<RecordBatch>>()
            .await
            .expect_err("MaxLenReachedError");
    }
}
