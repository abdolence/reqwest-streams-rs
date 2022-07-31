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
    fn json_nl_stream<'a, 'b, T>(self, max_obj_len: usize) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: for<'de> Deserialize<'de> + Send + 'b;
}

#[async_trait]
impl JsonStreamResponse for reqwest::Response {
    fn json_nl_stream<'a, 'b, T>(self, max_obj_len: usize) -> BoxStream<'b, StreamBodyResult<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        let reader = StreamReader::new(
            self.bytes_stream()
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err)),
        );

        let codec = tokio_util::codec::LinesCodec::new_with_max_length(max_obj_len);
        let frames_reader = tokio_util::codec::FramedRead::new(reader, codec);

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
        let reader = StreamReader::new(
            self.bytes_stream()
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err)),
        );

        //serde_json::from_reader(read);
        let codec = JsonArrayCodec::<T>::new_with_max_length(max_obj_len);
        let frames_reader = tokio_util::codec::FramedRead::new(reader, codec);

        Box::pin(frames_reader.into_stream())
    }
}
