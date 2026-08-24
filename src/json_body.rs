//! Streaming a JSON request body.

use crate::stream_body::{ReqwestStreamBody, StreamBodyRequest};
use futures::Stream;
use http_streams_core::{JsonArrayStreamFormat, JsonNewLineStreamFormat};
use serde::Serialize;

/// Extension trait for [`reqwest::RequestBuilder`] that streams a JSON request body.
///
/// The `try_` variants take a fallible source stream. See [`ReqwestStreamBody`] for the HTTP
/// caveats that apply to every streamed request body — the redirect one in particular.
pub trait JsonStreamRequest {
    /// Streams `stream` as a JSON array, setting `Content-Type: application/json`.
    ///
    /// ```rust,no_run
    /// use futures::stream;
    /// use reqwest_streams::JsonStreamRequest as _;
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct MyItem {
    ///     field: String,
    /// }
    ///
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let items = stream::iter(vec![MyItem { field: "value".into() }]);
    ///
    /// reqwest::Client::new()
    ///     .post("http://localhost:8080/ingest")
    ///     .json_array_stream_body(items)
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    fn json_array_stream_body<S, T>(self, stream: S) -> reqwest::RequestBuilder
    where
        T: Serialize + Send + 'static,
        S: Stream<Item = T> + Send + 'static;

    /// Streams a fallible `stream` as a JSON array.
    fn try_json_array_stream_body<S, T, E>(self, stream: S) -> reqwest::RequestBuilder
    where
        T: Serialize + Send + 'static,
        S: Stream<Item = Result<T, E>> + Send + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static;

    /// Streams `stream` as JSON Lines, setting `Content-Type: application/jsonstream`.
    fn json_nl_stream_body<S, T>(self, stream: S) -> reqwest::RequestBuilder
    where
        T: Serialize + Send + 'static,
        S: Stream<Item = T> + Send + 'static;

    /// Streams a fallible `stream` as JSON Lines.
    fn try_json_nl_stream_body<S, T, E>(self, stream: S) -> reqwest::RequestBuilder
    where
        T: Serialize + Send + 'static,
        S: Stream<Item = Result<T, E>> + Send + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static;
}

impl JsonStreamRequest for reqwest::RequestBuilder {
    fn json_array_stream_body<S, T>(self, stream: S) -> reqwest::RequestBuilder
    where
        T: Serialize + Send + 'static,
        S: Stream<Item = T> + Send + 'static,
    {
        self.stream_body(ReqwestStreamBody::new(JsonArrayStreamFormat::new(), stream))
    }

    fn try_json_array_stream_body<S, T, E>(self, stream: S) -> reqwest::RequestBuilder
    where
        T: Serialize + Send + 'static,
        S: Stream<Item = Result<T, E>> + Send + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static,
    {
        self.stream_body(ReqwestStreamBody::try_new(
            JsonArrayStreamFormat::new(),
            stream,
        ))
    }

    fn json_nl_stream_body<S, T>(self, stream: S) -> reqwest::RequestBuilder
    where
        T: Serialize + Send + 'static,
        S: Stream<Item = T> + Send + 'static,
    {
        self.stream_body(ReqwestStreamBody::new(
            JsonNewLineStreamFormat::new(),
            stream,
        ))
    }

    fn try_json_nl_stream_body<S, T, E>(self, stream: S) -> reqwest::RequestBuilder
    where
        T: Serialize + Send + 'static,
        S: Stream<Item = Result<T, E>> + Send + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static,
    {
        self.stream_body(ReqwestStreamBody::try_new(
            JsonNewLineStreamFormat::new(),
            stream,
        ))
    }
}
