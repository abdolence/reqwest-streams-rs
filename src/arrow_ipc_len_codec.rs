use crate::error::StreamBodyKind;
use crate::StreamBodyError;
use arrow::array::RecordBatch;
use arrow::ipc::reader::StreamDecoder;
use bytes::{Buf, BytesMut};

#[derive(Debug)]
pub struct ArrowIpcCodec {
    max_length: usize,
    decoder: StreamDecoder,
    current_obj_len: usize,
}

impl ArrowIpcCodec {
    pub fn new_with_max_length(max_length: usize) -> Self {
        ArrowIpcCodec {
            max_length,
            decoder: StreamDecoder::new(),
            current_obj_len: 0,
        }
    }
}

impl tokio_util::codec::Decoder for ArrowIpcCodec {
    type Item = RecordBatch;
    type Error = StreamBodyError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<RecordBatch>, StreamBodyError> {
        let buf_len = buf.len();
        if buf_len == 0 {
            return Ok(None);
        }

        let obj_bytes = buf.as_ref();
        let obj_bytes_len = obj_bytes.len();
        let mut buffer = arrow::buffer::Buffer::from(obj_bytes);
        let maybe_record = self.decoder.decode(&mut buffer).map_err(|e| {
            StreamBodyError::new(
                StreamBodyKind::CodecError,
                Some(Box::new(e)),
                Some("Decode arrow IPC record error".into()),
            )
        })?;

        if maybe_record.is_none() {
            self.current_obj_len += obj_bytes_len;
        } else {
            self.current_obj_len = 0;
        }

        if self.current_obj_len > self.max_length {
            return Err(StreamBodyError::new(
                StreamBodyKind::CodecError,
                None,
                Some("Object length exceeds the maximum length".into()),
            ));
        }

        buf.advance(obj_bytes_len - buffer.len());
        Ok(maybe_record)
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<RecordBatch>, StreamBodyError> {
        self.decode(buf)
    }
}
