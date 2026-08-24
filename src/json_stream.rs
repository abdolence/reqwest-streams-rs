use crate::observability::{self, INITIAL_CAPACITY};
use crate::{ReqwestStreamOptions, StreamBodyResult};
use async_trait::*;
use http_streams_core::{JsonArrayStreamFormat, JsonNewLineStreamFormat};
use serde::Deserialize;

/// Extension trait for [`reqwest::Response`] that provides streaming support for the JSON array
/// and JSON Lines (NL/NewLines) formats.
#[async_trait]
pub trait JsonStreamResponse {
    /// Streams the response as a JSON array.
    ///
    /// The stream will [`Deserialize`] entries as type `T` with a maximum size of `max_obj_len`
    /// bytes. If `max_obj_len` is [`usize::MAX`], lines will be read until a newline (`\n`)
    /// character is reached.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use futures::stream::BoxStream as _;
    /// use reqwest_streams::JsonStreamResponse as _;
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
    ///     let _stream = reqwest::get("http://localhost:8080/json-array")
    ///         .await?
    ///         .json_array_stream::<MyTestStructure>(MAX_OBJ_LEN);
    ///
    ///     Ok(())
    /// }
    /// ```
    fn json_array_stream<'a, 'b, T>(self, max_obj_len: usize) -> impl futures::Stream<Item = StreamBodyResult<T>>  + Send + 'b
    where
        T: for<'de> Deserialize<'de> + Send + 'b;

    /// Streams the response as a JSON array.
    ///
    /// The stream will [`Deserialize`] entries as type `T` with a maximum size of `max_obj_len`
    /// bytes. If `max_obj_len` is [`usize::MAX`], lines will be read until a newline (`\n`)
    /// character is reached.
    ///
    /// `buf_capacity` is the initial capacity of the stream's decoding buffer.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use futures::stream::BoxStream as _;
    /// use reqwest_streams::JsonStreamResponse as _;
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
    ///     const INITIAL_BUF_CAPACITY: usize = 16 * 1024;
    ///
    ///     let _stream = reqwest::get("http://localhost:8080/json-array")
    ///         .await?
    ///         .json_array_stream_with_capacity::<MyTestStructure>(MAX_OBJ_LEN, INITIAL_BUF_CAPACITY);
    ///
    ///     Ok(())
    /// }
    /// ```
    fn json_array_stream_with_capacity<'a, 'b, T>(
        self,
        max_obj_len: usize,
        buf_capacity: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>>  + Send + 'b
    where
        T: for<'de> Deserialize<'de> + Send + 'b;

    /// Streams the response as JSON lines (NL/NewLines), where each line contains a JSON object.
    ///
    /// The stream will [`Deserialize`] entries as type `T` with a maximum size of `max_obj_len`
    /// bytes. If `max_obj_len` is [`usize::MAX`], lines will be read until a newline (`\n`)
    /// character is reached.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use futures::stream::BoxStream as _;
    /// use reqwest_streams::JsonStreamResponse as _;
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
    ///     let _stream = reqwest::get("http://localhost:8080/json-nl")
    ///         .await?
    ///         .json_nl_stream::<MyTestStructure>(MAX_OBJ_LEN);
    ///
    ///     Ok(())
    /// }
    /// ```
    fn json_nl_stream<'a, 'b, T>(self, max_obj_len: usize) -> impl futures::Stream<Item = StreamBodyResult<T>>  + Send + 'b
    where
        T: for<'de> Deserialize<'de> + Send + 'b;

    /// Streams the response as JSON lines (NL/NewLines), where each line contains a JSON object.
    ///
    /// The stream will [`Deserialize`] entries as type `T` with a maximum size of `max_obj_len`
    /// bytes. If `max_obj_len` is [`usize::MAX`], lines will be read until a `\n` character
    /// is reached.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use futures::stream::BoxStream as _;
    /// use reqwest_streams::JsonStreamResponse as _;
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
    ///     const INITIAL_BUF_CAPACITY: usize = 16 * 1024;
    ///
    ///     let _stream = reqwest::get("http://localhost:8080/json-nl")
    ///         .await?
    ///         .json_nl_stream_with_capacity::<MyTestStructure>(MAX_OBJ_LEN, INITIAL_BUF_CAPACITY);
    ///
    ///     Ok(())
    /// }
    /// ```
    fn json_nl_stream_with_capacity<'a, 'b, T>(
        self,
        max_obj_len: usize,
        buf_capacity: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send + 'b
    where
        T: for<'de> Deserialize<'de> + Send + 'b;

    /// Streams the response as a JSON array, with [`ReqwestStreamOptions`].
    ///
    /// This is the variant that gives you the observability hooks: see
    /// [`ReqwestStreamOptions::on_error`] and [`ReqwestStreamOptions::on_progress`].
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use reqwest_streams::{JsonStreamResponse as _, ReqwestStreamOptions};
    /// use serde::Deserialize;
    ///
    /// #[derive(Debug, Clone, Deserialize)]
    /// struct MyTestStructure {
    ///     some_test_field: String
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let _stream = reqwest::get("http://localhost:8080/json-array")
    ///         .await?
    ///         .json_array_stream_with_options::<MyTestStructure>(
    ///             ReqwestStreamOptions::new()
    ///                 .max_obj_len(64 * 1024)
    ///                 .on_error(|err| eprintln!("stream error: {err}")),
    ///         );
    ///
    ///     Ok(())
    /// }
    /// ```
    fn json_array_stream_with_options<'a, 'b, T>(
        self,
        options: ReqwestStreamOptions,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send + 'b
    where
        T: for<'de> Deserialize<'de> + Send + 'b;

    /// Streams the response as JSON lines (NL/NewLines), with [`ReqwestStreamOptions`].
    ///
    /// This is the variant that gives you the observability hooks: see
    /// [`ReqwestStreamOptions::on_error`] and [`ReqwestStreamOptions::on_progress`].
    fn json_nl_stream_with_options<'a, 'b, T>(
        self,
        options: ReqwestStreamOptions,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send + 'b
    where
        T: for<'de> Deserialize<'de> + Send + 'b;
}

#[async_trait]
impl JsonStreamResponse for reqwest::Response {
    fn json_nl_stream<'a, 'b, T>(self, max_obj_len: usize) -> impl futures::Stream<Item = StreamBodyResult<T>>  + Send + 'b
    where
        T: for<'de> Deserialize<'de> + Send + 'b,
    {
        self.json_nl_stream_with_capacity(max_obj_len, INITIAL_CAPACITY)
    }

    fn json_nl_stream_with_capacity<'a, 'b, T>(
        self,
        max_obj_len: usize,
        buf_capacity: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>>  + Send + 'b
    where
        T: for<'de> Deserialize<'de> + Send + 'b
    {
        self.json_nl_stream_with_options(
            ReqwestStreamOptions::new()
                .max_obj_len(max_obj_len)
                .buf_capacity(buf_capacity),
        )
    }

    fn json_nl_stream_with_options<'a, 'b, T>(
        self,
        options: ReqwestStreamOptions,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send + 'b
    where
        T: for<'de> Deserialize<'de> + Send + 'b,
    {
        observability::decode_response(
            self,
            JsonNewLineStreamFormat::new(),
            "json_nl",
            options,
        )
    }

    fn json_array_stream<'a, 'b, T>(self, max_obj_len: usize) -> impl futures::Stream<Item = StreamBodyResult<T>>  + Send + 'b
    where
        T: for<'de> Deserialize<'de> + Send + 'b,
    {
        self.json_array_stream_with_capacity(max_obj_len, INITIAL_CAPACITY)
    }

    fn json_array_stream_with_capacity<'a, 'b, T>(
        self,
        max_obj_len: usize,
        buf_capacity: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>>  + Send + 'b
    where
        T: for<'de> Deserialize<'de> + Send + 'b,
    {
        self.json_array_stream_with_options(
            ReqwestStreamOptions::new()
                .max_obj_len(max_obj_len)
                .buf_capacity(buf_capacity),
        )
    }

    fn json_array_stream_with_options<'a, 'b, T>(
        self,
        options: ReqwestStreamOptions,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send + 'b
    where
        T: for<'de> Deserialize<'de> + Send + 'b,
    {
        observability::decode_response(
            self,
            JsonArrayStreamFormat::new(),
            "json_array",
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

    #[tokio::test]
    async fn deserialize_json_array_stream_primitives_i32() {
        let test_stream_vec: Vec<i32> = (1..=10).collect();

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route("/", get(|| async { StreamBodyAs::json_array(test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .json_array_stream::<i32>(1024);
        let items: Vec<i32> = res.try_collect().await.unwrap();

        assert_eq!(items, test_stream_vec);
    }

    #[tokio::test]
    async fn deserialize_json_array_stream_primitives_string() {
        let test_stream_vec: Vec<String> = vec!["hello".into(), "world".into(), r#"has\"quote"#.into()];

        let test_stream = Box::pin(stream::iter(test_stream_vec.clone()));

        let app = Router::new().route("/", get(|| async { StreamBodyAs::json_array(test_stream) }));

        let client = TestClient::new(app).await;

        let res = client
            .get("/")
            .send()
            .await
            .unwrap()
            .json_array_stream::<String>(1024);
        let items: Vec<String> = res.try_collect().await.unwrap();

        assert_eq!(items, test_stream_vec);
    }

    #[tokio::test]
    async fn deserialize_json_array_stream_string_with_trailing_backslash() {
        let test_stream_vec = vec![
            MyTestStructure {
                some_test_field: r#"TestValue"\"#.to_string(),
                test_arr: vec![]
            };
            100
        ];

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
}
