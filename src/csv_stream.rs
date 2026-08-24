use crate::observability;
use crate::{ReqwestStreamOptions, StreamBodyResult};
use async_trait::*;
use http_streams_core::CsvStreamFormat;
use serde::Deserialize;

/// Extension trait for [`reqwest::Response`] that provides streaming support for the CSV format.
#[async_trait]
pub trait CsvStreamResponse {
    /// Streams the response as CSV, where each line is a CSV row.
    ///
    /// The stream will [`Deserialize`] entries as type `T` with a maximum size of `max_obj_len`
    /// bytes. If `max_obj_len` is [`usize::MAX`], lines will be read until a newline (`\n`)
    /// character is reached.
    ///
    /// If `with_csv_header` is `true`, the stream will skip the first row (the CSV header).
    ///
    /// The `delimiter` is the byte value of the delimiter character.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use futures::stream::BoxStream as _;
    /// use reqwest_streams::CsvStreamResponse as _;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Debug, Clone, Deserialize)]
    /// struct MyTestStructure {
    ///     some_test_field: String
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     const MAX_OBJ_LEN: usize = 64 * 1024;
    ///
    ///     let _stream = reqwest::get("http://localhost:8080/csv")
    ///         .await?
    ///         .csv_stream::<MyTestStructure>(MAX_OBJ_LEN, true, b',');
    ///
    ///     Ok(())
    /// }
    /// ```
    fn csv_stream<'a, 'b, T>(
        self,
        max_obj_len: usize,
        with_csv_header: bool,
        delimiter: u8,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>>  + Send + 'b
    where
        T: for<'de> Deserialize<'de>;

    /// Streams the response as CSV, with [`ReqwestStreamOptions`].
    ///
    /// `with_csv_header` and `delimiter` stay here rather than moving into the options because
    /// they describe the CSV format itself, not how the stream is read.
    ///
    /// This is the variant that gives you the observability hooks: see
    /// [`ReqwestStreamOptions::on_error`] and [`ReqwestStreamOptions::on_progress`].
    fn csv_stream_with_options<'a, 'b, T>(
        self,
        with_csv_header: bool,
        delimiter: u8,
        options: ReqwestStreamOptions,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send + 'b
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
    ) -> impl futures::Stream<Item = StreamBodyResult<T>>  + Send + 'b
    where
        T: for<'de> Deserialize<'de>,
    {
        self.csv_stream_with_options(
            with_csv_header,
            delimiter,
            ReqwestStreamOptions::new().max_obj_len(max_obj_len),
        )
    }

    fn csv_stream_with_options<'a, 'b, T>(
        self,
        with_csv_header: bool,
        delimiter: u8,
        options: ReqwestStreamOptions,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send + 'b
    where
        T: for<'de> Deserialize<'de>,
    {
        // The header slot is consumed by the codec rather than with a `.skip(1)`, which would
        // drop the first frame whether it decoded or not: a header line that failed to frame —
        // one longer than `max_obj_len`, say — would be swallowed, and the stream would report
        // itself as having completed cleanly.
        observability::decode_response(
            self,
            CsvStreamFormat::new(with_csv_header, delimiter),
            "csv",
            options,
        )
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
            // `StreamBodyAs::csv` writes a header row, so the client has to skip one.
            .csv_stream::<MyTestStructure>(1024, true, b',');
        let items: Vec<MyTestStructure> = res.try_collect().await.unwrap();

        assert_eq!(items, test_stream_vec);
    }

    #[tokio::test]
    async fn deserialize_csv_stream_with_header() {
        let test_stream_vec = generate_test_structures();

        let test_stream = Box::pin(stream::iter(
            test_stream_vec
                .clone()
                .into_iter()
                .map(Ok::<_, axum::Error>),
        ));

        let app = Router::new().route(
            "/",
            get(|| async { StreamBodyAs::new(axum_streams::CsvStreamFormat::new(true, b','), test_stream) }),
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

        // Serves CSV, not a JSON array: decoding a JSON body as CSV would error for the wrong
        // reason and the test would pass without exercising the length limit at all.
        let app = Router::new().route("/", get(|| async { StreamBodyAs::csv(test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .csv_stream::<MyTestStructure>(5, true, b',');
        let err = res
            .try_collect::<Vec<MyTestStructure>>()
            .await
            .expect_err("a row longer than the limit must fail");
        assert_eq!(
            err.kind(),
            crate::error::StreamBodyKind::MaxLenReachedError,
            "it must fail because of the length limit, not for some other reason"
        );
    }
}
