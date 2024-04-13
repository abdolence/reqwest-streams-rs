use arrow::array::RecordBatch;
use arrow::ipc::reader::StreamDecoder;
use crate::StreamBodyError;
use bytes::{Buf, Bytes, BytesMut};
use crate::error::StreamBodyKind;


#[derive(Debug)]
pub struct ArrowIpcCodec {
    max_length: usize,
    cursor: ArrowIpcCursor,
    decoder: StreamDecoder
}

#[derive(Clone, Debug)]
struct ArrowIpcCursor {
    current_obj_len: usize,
}

impl ArrowIpcCodec {
    pub fn new_with_max_length(max_length: usize) -> Self {
        let initial_cursor = ArrowIpcCursor { current_obj_len: 0 };

        ArrowIpcCodec {
            max_length,
            cursor: initial_cursor,
            decoder: StreamDecoder::new()
        }
    }
}

const IPC_STREAM_CONTINUATION_MARKER: u32 = 0xffffffff;

impl tokio_util::codec::Decoder for ArrowIpcCodec {
    type Item = RecordBatch;
    type Error = StreamBodyError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<RecordBatch>, StreamBodyError> {
        let buf_len = buf.len();
        if buf_len == 0 {
            return Ok(None);
        }
        if self.cursor.current_obj_len == 0 {
            if buf_len < 8 {
                Ok(None) // wait more bytes for len
            } else {
                let len_buf_ptr = buf.get(0..8).expect("Buf len is 8, so this must work");
                let mut len_buf: Bytes = Bytes::copy_from_slice(len_buf_ptr);
                let mut possible_len = len_buf.get_u32_le();
                if(possible_len == IPC_STREAM_CONTINUATION_MARKER) {
                    possible_len = len_buf.get_u32_le() + 4;
                }
                self.cursor.current_obj_len = (possible_len + 4) as usize;
                Ok(None)
            }
        }
        else if self.cursor.current_obj_len > self.max_length {
            Err(StreamBodyError::new(
                StreamBodyKind::MaxLenReachedError,
                None,
                Some("Max object length reached".into()),
            ))
        } else if buf_len >= self.cursor.current_obj_len {
            let obj_bytes = buf.copy_to_bytes(self.cursor.current_obj_len);
            self.cursor.current_obj_len = 0;
            let mut buffer = arrow::buffer::Buffer::from(&obj_bytes);
            let maybe_record = self.decoder.decode(&mut buffer).map_err(|e| StreamBodyError::new(
                StreamBodyKind::CodecError,
                Some(Box::new(e)),
                Some("Decode arrow IPC record error".into())
            ))?;
            Ok(maybe_record)
        } else {
            Ok(None)
        }
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<RecordBatch>, StreamBodyError> {
        self.decode(buf)
    }
}
