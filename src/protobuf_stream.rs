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
