use crate::error::StreamBodyKind;
use crate::{StreamBodyError, StreamBodyResult};
use async_trait::*;
use futures_util::stream::BoxStream;
use futures_util::{StreamExt, TryStreamExt};
use serde::Deserialize;
use tokio_util::io::StreamReader;

#[async_trait]
pub trait CsvStreamResponse {
    fn csv_stream<'a, 'b, T>(
        self,
        max_obj_len: usize,
        with_csv_header: bool,
        delimiter: u8,
    ) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: for<'de> Deserialize<'de>;
}

#[async_trait]
impl CsvStreamResponse for reqwest::Response {
    fn csv_stream<'a, 'b, T>(
        self,
        max_obj_len: usize,
        with_csv_header: bool,
        delimiter: u8,
    ) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        let reader = StreamReader::new(
            self.bytes_stream()
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err)),
        );

        let codec = tokio_util::codec::LinesCodec::new_with_max_length(max_obj_len);
        let frames_reader = tokio_util::codec::FramedRead::new(reader, codec);

        #[allow(clippy::bool_to_int_with_if)] // false positive: it is not bool to int
        let skip_header_if_expected = if with_csv_header { 1 } else { 0 };

        Box::pin(
            frames_reader
                .into_stream()
                .skip(skip_header_if_expected)
                .map(move |frame_res| match frame_res {
                    Ok(frame_str) => {
                        let mut csv_reader = csv::ReaderBuilder::new()
                            .delimiter(delimiter)
                            .has_headers(false)
                            .from_reader(frame_str.as_bytes());

                        let mut iter = csv_reader.deserialize::<T>();

                        if let Some(csv_res) = iter.next() {
                            match csv_res {
                                Ok(result) => Ok(result),
                                Err(err) => Err(StreamBodyError::new(
                                    StreamBodyKind::CodecError,
                                    Some(Box::new(err)),
                                    None,
                                )),
                            }
                        } else {
                            Err(StreamBodyError::new(StreamBodyKind::CodecError, None, None))
                        }
                    }
                    Err(err) => Err(StreamBodyError::new(
                        StreamBodyKind::CodecError,
                        Some(Box::new(err)),
                        None,
                    )),
                }),
        )
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
        some_test_field1: String,
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
    async fn deserialize_csv_stream() {
        let test_stream_vec = generate_test_structures();

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route("/", get(|| async { StreamBodyAs::csv(test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .csv_stream::<MyTestStructure>(1024, false, b',');
        let items: Vec<MyTestStructure> = res.try_collect().await.unwrap();

        assert_eq!(items, test_stream_vec);
    }

    #[tokio::test]
    async fn deserialize_csv_stream_with_header() {
        let test_stream_vec = generate_test_structures();

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route(
            "/",
            get(|| async { StreamBodyAs::new(CsvStreamFormat::new(true, b','), test_stream) }),
        );

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .csv_stream::<MyTestStructure>(1024, true, b',');
        let items: Vec<MyTestStructure> = res.try_collect().await.unwrap();

        assert_eq!(items, test_stream_vec);
    }

    #[tokio::test]
    async fn deserialize_csv_check_max_len() {
        let test_stream_vec = generate_test_structures();

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route("/", get(|| async { StreamBodyAs::json_array(test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .csv_stream::<MyTestStructure>(5, false, b',');
        res.try_collect::<Vec<MyTestStructure>>()
            .await
            .expect_err("MaxLenReachedError");
    }
}
