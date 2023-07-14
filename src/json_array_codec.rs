use crate::error::StreamBodyKind;
use crate::StreamBodyError;
use bytes::{Buf, BytesMut};
use serde::Deserialize;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub struct JsonArrayCodec<T> {
    max_length: usize,
    json_cursor: JsonCursor,
    _ph: PhantomData<T>,
}

#[derive(Clone, Debug)]
struct JsonCursor {
    pub current_offset: usize,
    pub array_is_opened: bool,
    pub delimiter_expected: bool,
    pub quote_opened: bool,
    pub escaped: bool,
    pub opened_brackets: usize,
    pub current_obj_pos: usize,
}

impl<T> JsonArrayCodec<T> {
    pub fn new_with_max_length(max_length: usize) -> Self {
        let initial_cursor = JsonCursor {
            current_offset: 0,
            array_is_opened: false,
            delimiter_expected: false,
            quote_opened: false,
            escaped: false,
            opened_brackets: 0,
            current_obj_pos: 0,
        };

        JsonArrayCodec {
            max_length,
            json_cursor: initial_cursor,
            _ph: PhantomData,
        }
    }
}

impl<T> tokio_util::codec::Decoder for JsonArrayCodec<T>
where
    T: for<'de> Deserialize<'de>,
{
    type Item = T;
    type Error = StreamBodyError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<T>, StreamBodyError> {
        if buf.is_empty() {
            return Ok(None);
        }

        for (position, current_ch) in buf[self.json_cursor.current_offset..buf.len()]
            .iter()
            .enumerate()
        {
            if self.json_cursor.current_offset + position >= self.max_length {
                return Err(StreamBodyError::new(
                    StreamBodyKind::MaxLenReachedError,
                    None,
                    Some("Max object length reached".into()),
                ));
            }
            match *current_ch {
                b'[' if !self.json_cursor.quote_opened && self.json_cursor.opened_brackets == 0 => {
                    if self.json_cursor.array_is_opened {
                        return Err(StreamBodyError::new(
                            StreamBodyKind::CodecError,
                            None,
                            Some("Unexpected array begin. It is already opened".into()),
                        ));
                    } else {
                        self.json_cursor.array_is_opened = true;
                    }
                }
                b'"' if !self.json_cursor.escaped => {
                    self.json_cursor.quote_opened = !self.json_cursor.quote_opened;
                }
                b'\\' if self.json_cursor.quote_opened => {
                    self.json_cursor.escaped = true;
                }
                b'{' if !self.json_cursor.quote_opened => {
                    if self.json_cursor.opened_brackets == 0 {
                        self.json_cursor.current_obj_pos =
                            self.json_cursor.current_offset + position;
                    }
                    self.json_cursor.opened_brackets += 1;
                    self.json_cursor.escaped = false;
                }
                b'}' if !self.json_cursor.quote_opened => {
                    self.json_cursor.opened_brackets -= 1;
                    self.json_cursor.escaped = false;
                    if self.json_cursor.opened_brackets == 0 {
                        self.json_cursor.delimiter_expected = true;
                        let obj_slice = &buf[self.json_cursor.current_obj_pos
                            ..self.json_cursor.current_offset + position + 1];
                        let result = serde_json::from_slice(obj_slice).map_err(|err| {
                            StreamBodyError::new(
                                StreamBodyKind::CodecError,
                                Some(Box::new(err)),
                                None,
                            )
                        });
                        self.json_cursor.current_obj_pos = 0;
                        buf.advance(self.json_cursor.current_offset + position + 1);
                        self.json_cursor.current_offset = 0;
                        return result;
                    }
                }
                b',' if !self.json_cursor.quote_opened
                    && self.json_cursor.opened_brackets == 0
                    && !self.json_cursor.delimiter_expected =>
                {
                    return Err(StreamBodyError::new(
                        StreamBodyKind::CodecError,
                        None,
                        Some("Unexpected delimiter found".into()),
                    ));
                }
                _ => {
                    self.json_cursor.escaped = false;
                }
            }
        }
        self.json_cursor.current_offset = buf.len();

        Ok(None)
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<T>, StreamBodyError> {
        self.decode(buf)
    }
}
