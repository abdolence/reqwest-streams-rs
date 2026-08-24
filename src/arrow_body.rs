//! Streaming an Apache Arrow IPC request body.

use crate::stream_body::{ReqwestStreamBody, StreamBodyRequest};
use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use futures::Stream;
use http_streams_core::ArrowRecordBatchIpcStreamFormat;

/// Extension trait for [`reqwest::RequestBuilder`] that streams an Arrow IPC request body.
///
/// Unlike decoding, encoding needs the schema up front: it is written once, ahead of the first
/// batch. See [`ReqwestStreamBody`] for the HTTP caveats.
pub trait ArrowIpcStreamRequest {
    /// Streams `stream` as an Arrow IPC stream, setting
    /// `Content-Type: application/vnd.apache.arrow.stream`.
    fn arrow_ipc_stream_body<S>(self, schema: SchemaRef, stream: S) -> reqwest::RequestBuilder
    where
        S: Stream<Item = RecordBatch> + Send + 'static;

    /// Streams a fallible `stream` as an Arrow IPC stream.
    fn try_arrow_ipc_stream_body<S, E>(
        self,
        schema: SchemaRef,
        stream: S,
    ) -> reqwest::RequestBuilder
    where
        S: Stream<Item = Result<RecordBatch, E>> + Send + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static;
}

impl ArrowIpcStreamRequest for reqwest::RequestBuilder {
    fn arrow_ipc_stream_body<S>(self, schema: SchemaRef, stream: S) -> reqwest::RequestBuilder
    where
        S: Stream<Item = RecordBatch> + Send + 'static,
    {
        self.stream_body(ReqwestStreamBody::new(
            ArrowRecordBatchIpcStreamFormat::new(schema),
            stream,
        ))
    }

    fn try_arrow_ipc_stream_body<S, E>(
        self,
        schema: SchemaRef,
        stream: S,
    ) -> reqwest::RequestBuilder
    where
        S: Stream<Item = Result<RecordBatch, E>> + Send + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static,
    {
        self.stream_body(ReqwestStreamBody::try_new(
            ArrowRecordBatchIpcStreamFormat::new(schema),
            stream,
        ))
    }
}
