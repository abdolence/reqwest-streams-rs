//! Streaming a CSV request body.

use crate::stream_body::{ReqwestStreamBody, StreamBodyRequest};
use futures::Stream;
use http_streams_core::CsvStreamFormat;
use serde::Serialize;

/// Extension trait for [`reqwest::RequestBuilder`] that streams a CSV request body.
///
/// See [`ReqwestStreamBody`] for the HTTP caveats that apply to every streamed request body.
pub trait CsvStreamRequest {
    /// Streams `stream` as CSV, setting `Content-Type: text/csv`.
    ///
    /// `with_csv_header` writes a header row from the field names before the first record.
    fn csv_stream_body<S, T>(
        self,
        stream: S,
        with_csv_header: bool,
        delimiter: u8,
    ) -> reqwest::RequestBuilder
    where
        T: Serialize + Send + 'static,
        S: Stream<Item = T> + Send + 'static;

    /// Streams a fallible `stream` as CSV.
    fn try_csv_stream_body<S, T, E>(
        self,
        stream: S,
        with_csv_header: bool,
        delimiter: u8,
    ) -> reqwest::RequestBuilder
    where
        T: Serialize + Send + 'static,
        S: Stream<Item = Result<T, E>> + Send + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static;
}

impl CsvStreamRequest for reqwest::RequestBuilder {
    fn csv_stream_body<S, T>(
        self,
        stream: S,
        with_csv_header: bool,
        delimiter: u8,
    ) -> reqwest::RequestBuilder
    where
        T: Serialize + Send + 'static,
        S: Stream<Item = T> + Send + 'static,
    {
        self.stream_body(ReqwestStreamBody::new(
            CsvStreamFormat::new(with_csv_header, delimiter),
            stream,
        ))
    }

    fn try_csv_stream_body<S, T, E>(
        self,
        stream: S,
        with_csv_header: bool,
        delimiter: u8,
    ) -> reqwest::RequestBuilder
    where
        T: Serialize + Send + 'static,
        S: Stream<Item = Result<T, E>> + Send + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static,
    {
        self.stream_body(ReqwestStreamBody::try_new(
            CsvStreamFormat::new(with_csv_header, delimiter),
            stream,
        ))
    }
}
