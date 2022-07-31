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

        Box::pin(
            frames_reader.into_stream().enumerate().map(
                move |(index, frame_res)| match frame_res {
                    Ok(frame_str) => {
                        let mut csv_reader = if index == 0 {
                            csv::ReaderBuilder::new()
                                .delimiter(delimiter)
                                .has_headers(with_csv_header)
                                .from_reader(frame_str.as_bytes())
                        } else {
                            csv::ReaderBuilder::new()
                                .delimiter(delimiter)
                                .has_headers(false)
                                .from_reader(frame_str.as_bytes())
                        };

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
                },
            ),
        )
    }
}
