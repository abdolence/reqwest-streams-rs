//! Streaming a length-prefixed protobuf request body.

use crate::stream_body::{ReqwestStreamBody, StreamBodyRequest};
use futures::Stream;
use http_streams_core::ProtobufStreamFormat;

/// Extension trait for [`reqwest::RequestBuilder`] that streams a protobuf request body.
///
/// See [`ReqwestStreamBody`] for the HTTP caveats that apply to every streamed request body.
pub trait ProtobufStreamRequest {
    /// Streams `stream` as length-prefixed protobuf messages, setting
    /// `Content-Type: application/x-protobuf-stream`.
    fn protobuf_stream_body<S, T>(self, stream: S) -> reqwest::RequestBuilder
    where
        T: prost::Message + Send + 'static,
        S: Stream<Item = T> + Send + 'static;

    /// Streams a fallible `stream` as length-prefixed protobuf messages.
    fn try_protobuf_stream_body<S, T, E>(self, stream: S) -> reqwest::RequestBuilder
    where
        T: prost::Message + Send + 'static,
        S: Stream<Item = Result<T, E>> + Send + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static;
}

impl ProtobufStreamRequest for reqwest::RequestBuilder {
    fn protobuf_stream_body<S, T>(self, stream: S) -> reqwest::RequestBuilder
    where
        T: prost::Message + Send + 'static,
        S: Stream<Item = T> + Send + 'static,
    {
        self.stream_body(ReqwestStreamBody::new(ProtobufStreamFormat::new(), stream))
    }

    fn try_protobuf_stream_body<S, T, E>(self, stream: S) -> reqwest::RequestBuilder
    where
        T: prost::Message + Send + 'static,
        S: Stream<Item = Result<T, E>> + Send + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static,
    {
        self.stream_body(ReqwestStreamBody::try_new(
            ProtobufStreamFormat::new(),
            stream,
        ))
    }
}
