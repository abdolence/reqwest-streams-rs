use crate::protobuf_len_codec::ProtobufLenPrefixCodec;

use crate::StreamBodyResult;
use async_trait::*;
use futures_util::stream::BoxStream;
use futures_util::TryStreamExt;
use tokio_util::io::StreamReader;

#[async_trait]
pub trait ProtobufStreamResponse {
    fn protobuf_stream<'a, 'b, T>(self, max_obj_len: usize) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: prost::Message + Default + Send + 'b;
}

#[async_trait]
impl ProtobufStreamResponse for reqwest::Response {
    fn protobuf_stream<'a, 'b, T>(self, max_obj_len: usize) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: prost::Message + Default + Send + 'b,
    {
        let reader = StreamReader::new(
            self.bytes_stream()
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err)),
        );

        let codec = ProtobufLenPrefixCodec::<T>::new_with_max_length(max_obj_len);
        let frames_reader = tokio_util::codec::FramedRead::new(reader, codec);

        Box::pin(frames_reader.into_stream())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_client::*;
    use axum::{routing::*, Router};
    use axum_streams::*;
    use futures_util::stream;

    #[derive(Clone, prost::Message, PartialEq, Eq)]
    struct MyTestStructure {
        #[prost(string, tag = "1")]
        some_test_field1: String,
        #[prost(string, tag = "2")]
        some_test_field2: String,
    }

    fn generate_test_structures() -> Vec<MyTestStructure> {
        vec![
            MyTestStructure {
                some_test_field1: "TestValue1".to_string(),
                some_test_field2: "TestValue2".to_string()
            };
            100
        ]
    }

    #[tokio::test]
    async fn deserialize_proto_stream() {
        let test_stream_vec = generate_test_structures();

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route("/", get(|| async { StreamBodyAs::protobuf(test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .protobuf_stream::<MyTestStructure>(1024);
        let items: Vec<MyTestStructure> = res.try_collect().await.unwrap();

        assert_eq!(items, test_stream_vec);
    }

    #[tokio::test]
    async fn deserialize_proto_stream_check_max_len() {
        let test_stream_vec = generate_test_structures();

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route("/", get(|| async { StreamBodyAs::protobuf(test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .protobuf_stream::<MyTestStructure>(10);
        res.try_collect::<Vec<MyTestStructure>>()
            .await
            .expect_err("MaxLenReachedError");
    }
}
