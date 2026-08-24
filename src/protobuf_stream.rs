use crate::observability;
use crate::{ReqwestStreamOptions, StreamBodyResult};
use async_trait::*;
use http_streams_core::ProtobufStreamFormat;

/// Extension trait for [`reqwest::Response`] that provides streaming support for the [Protobuf
/// format].
///
/// [Protobuf format]: https://protobuf.dev/programming-guides/encoding/
#[async_trait]
pub trait ProtobufStreamResponse {
    /// Streams the response as batches of Protobuf messages.
    ///
    /// The stream will deserialize [`prost::Message`]s as type `T` with a maximum size of
    /// `max_obj_len` bytes.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use futures::{prelude::*, stream::BoxStream as _};
    /// use reqwest_streams::ProtobufStreamResponse as _;
    ///
    /// #[derive(Clone, prost::Message)]
    /// struct MyTestStructure {
    ///     #[prost(string, tag = "1")]
    ///     some_test_field: String,
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     const MAX_OBJ_LEN: usize = 64 * 1024;
    ///
    ///     let stream = reqwest::get("http://localhost:8080/protobuf")
    ///         .await?
    ///         .protobuf_stream::<MyTestStructure>(MAX_OBJ_LEN);
    ///     let _items: Vec<MyTestStructure> = stream.try_collect().await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    fn protobuf_stream<'a, 'b, T>(self, max_obj_len: usize) -> impl futures::Stream<Item = StreamBodyResult<T>>  + Send + 'b
    where
        T: prost::Message + Default + Send + 'b;

    /// Streams the response as batches of Protobuf messages, with [`ReqwestStreamOptions`].
    ///
    /// This is the variant that gives you the observability hooks: see
    /// [`ReqwestStreamOptions::on_error`] and [`ReqwestStreamOptions::on_progress`].
    fn protobuf_stream_with_options<'a, 'b, T>(
        self,
        options: ReqwestStreamOptions,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send + 'b
    where
        T: prost::Message + Default + Send + 'b;
}

#[async_trait]
impl ProtobufStreamResponse for reqwest::Response {
    fn protobuf_stream<'a, 'b, T>(self, max_obj_len: usize) -> impl futures::Stream<Item = StreamBodyResult<T>>  + Send + 'b
    where
        T: prost::Message + Default + Send + 'b,
    {
        self.protobuf_stream_with_options(ReqwestStreamOptions::new().max_obj_len(max_obj_len))
    }

    fn protobuf_stream_with_options<'a, 'b, T>(
        self,
        options: ReqwestStreamOptions,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send + 'b
    where
        T: prost::Message + Default + Send + 'b,
    {
        observability::decode_response(self, ProtobufStreamFormat::new(), "protobuf", options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use crate::test_client::*;
    use axum::{routing::*, Router};
    use axum_streams::*;
    use futures::stream;

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
