use crate::error::StreamBodyKind;
use crate::json_array_codec::JsonArrayCodec;
use crate::{StreamBodyError, StreamBodyResult};
use async_trait::*;
use futures_util::stream::BoxStream;
use futures_util::{StreamExt, TryStreamExt};
use serde::Deserialize;
use tokio_util::io::StreamReader;

#[async_trait]
pub trait JsonStreamResponse {
    fn json_array_stream<'a, 'b, T>(self, max_obj_len: usize) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: for<'de> Deserialize<'de> + Send + 'b;

    fn json_array_stream_with_capacity<'a, 'b, T>(
        self,
        max_obj_len: usize,
        buf_capacity: usize,
    ) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: for<'de> Deserialize<'de> + Send + 'b;

    fn json_nl_stream<'a, 'b, T>(self, max_obj_len: usize) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: for<'de> Deserialize<'de> + Send + 'b;

    fn json_nl_stream_with_capacity<'a, 'b, T>(
        self,
        max_obj_len: usize,
        buf_capacity: usize,
    ) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: for<'de> Deserialize<'de> + Send + 'b;
}

// This is the default capacity of the buffer used by StreamReader
const INITIAL_CAPACITY: usize = 8 * 1024;

#[async_trait]
impl JsonStreamResponse for reqwest::Response {
    fn json_nl_stream<'a, 'b, T>(self, max_obj_len: usize) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: for<'de> Deserialize<'de> + Send + 'b,
    {
        self.json_nl_stream_with_capacity(max_obj_len, INITIAL_CAPACITY)
    }

    fn json_nl_stream_with_capacity<'a, 'b, T>(
        self,
        max_obj_len: usize,
        buf_capacity: usize,
    ) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: for<'de> Deserialize<'de> + Send + 'b,
    {
        let reader = StreamReader::new(
            self.bytes_stream()
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err)),
        );

        let codec = tokio_util::codec::LinesCodec::new_with_max_length(max_obj_len);
        let frames_reader =
            tokio_util::codec::FramedRead::with_capacity(reader, codec, buf_capacity);

        Box::pin(
            frames_reader
                .into_stream()
                .map(|frame_res| match frame_res {
                    Ok(frame_str) => serde_json::from_str(frame_str.as_str()).map_err(|err| {
                        StreamBodyError::new(StreamBodyKind::CodecError, Some(Box::new(err)), None)
                    }),
                    Err(err) => Err(StreamBodyError::new(
                        StreamBodyKind::CodecError,
                        Some(Box::new(err)),
                        None,
                    )),
                }),
        )
    }

    fn json_array_stream<'a, 'b, T>(self, max_obj_len: usize) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: for<'de> Deserialize<'de> + Send + 'b,
    {
        self.json_array_stream_with_capacity(max_obj_len, INITIAL_CAPACITY)
    }

    fn json_array_stream_with_capacity<'a, 'b, T>(
        self,
        max_obj_len: usize,
        buf_capacity: usize,
    ) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: for<'de> Deserialize<'de> + Send + 'b,
    {
        let reader = StreamReader::new(
            self.bytes_stream()
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err)),
        );

        //serde_json::from_reader(read);
        let codec = JsonArrayCodec::<T>::new_with_max_length(max_obj_len);
        let frames_reader =
            tokio_util::codec::FramedRead::with_capacity(reader, codec, buf_capacity);

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
    use serde::Serialize;

    #[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
    struct MyTestStructure {
        some_test_field: String,
        test_arr: Vec<MyChildTest>,
    }

    #[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
    struct MyChildTest {
        test_field: String,
    }

    fn generate_test_structures() -> Vec<MyTestStructure> {
        vec![
            MyTestStructure {
                some_test_field: "TestValue".to_string(),
                test_arr: vec![
                    MyChildTest {
                        test_field: "TestValue1".to_string()
                    },
                    MyChildTest {
                        test_field: "TestValue2".to_string()
                    }
                ]
                .iter()
                .cloned()
                .collect()
            };
            100
        ]
    }

    #[tokio::test]
    async fn deserialize_json_array_stream() {
        let test_stream_vec = generate_test_structures();

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route("/", get(|| async { StreamBodyAs::json_array(test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .json_array_stream::<MyTestStructure>(1024);
        let items: Vec<MyTestStructure> = res.try_collect().await.unwrap();

        assert_eq!(items, test_stream_vec);
    }

    #[tokio::test]
    async fn deserialize_json_array_stream_check_max_len() {
        let test_stream_vec = generate_test_structures();

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route("/", get(|| async { StreamBodyAs::json_array(test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .json_array_stream::<MyTestStructure>(10);
        res.try_collect::<Vec<MyTestStructure>>()
            .await
            .expect_err("MaxLenReachedError");
    }

    #[tokio::test]
    async fn deserialize_json_array_stream_check_len_capacity() {
        let test_stream_vec = generate_test_structures();

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route("/", get(|| async { StreamBodyAs::json_array(test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .json_array_stream_with_capacity::<MyTestStructure>(1024, 50);

        let items: Vec<MyTestStructure> = res.try_collect().await.unwrap();

        assert_eq!(items, test_stream_vec);
    }

    #[tokio::test]
    async fn deserialize_json_nl_stream() {
        let test_stream_vec = generate_test_structures();

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route("/", get(|| async { StreamBodyAs::json_nl(test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .json_nl_stream::<MyTestStructure>(1024);
        let items: Vec<MyTestStructure> = res.try_collect().await.unwrap();

        assert_eq!(items, test_stream_vec);
    }

    #[tokio::test]
    async fn deserialize_json_nl_stream_check_max_len() {
        let test_stream_vec = generate_test_structures();

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route("/", get(|| async { StreamBodyAs::json_nl(test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .json_nl_stream::<MyTestStructure>(10);
        res.try_collect::<Vec<MyTestStructure>>()
            .await
            .expect_err("MaxLenReachedError");
    }
}
