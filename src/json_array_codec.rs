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
    /// When Some(pos), we are accumulating a primitive value (number/bool/null/string)
    /// that started at `pos` in the buffer. A quoted string also uses this.
    pub current_primitive_start: Option<usize>,
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
            current_primitive_start: None,
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
            let abs_pos = self.json_cursor.current_offset + position;

            if abs_pos >= self.max_length {
                return Err(StreamBodyError::new(
                    StreamBodyKind::MaxLenReachedError,
                    None,
                    Some("Max object length reached".into()),
                ));
            }

            match *current_ch {
                b'[' if !self.json_cursor.quote_opened && self.json_cursor.opened_brackets == 0 => {
                    if self.json_cursor.array_is_opened {
                        // This is a nested array item — treat like an object open
                        self.json_cursor.current_obj_pos = abs_pos;
                        self.json_cursor.opened_brackets += 1;
                        self.json_cursor.current_primitive_start = None;
                    } else {
                        self.json_cursor.array_is_opened = true;
                    }
                }
                b'[' if !self.json_cursor.quote_opened && self.json_cursor.opened_brackets > 0 => {
                    self.json_cursor.opened_brackets += 1;
                    self.json_cursor.escaped = false;
                }
                b']' if !self.json_cursor.quote_opened && self.json_cursor.opened_brackets == 0 => {
                    // End of the top-level array. Emit any pending primitive.
                    if let Some(prim_start) = self.json_cursor.current_primitive_start.take() {
                        let obj_slice = trim_ascii(&buf[prim_start..abs_pos]);
                        if !obj_slice.is_empty() {
                            let result = serde_json::from_slice(obj_slice).map_err(|err| {
                                StreamBodyError::new(
                                    StreamBodyKind::CodecError,
                                    Some(Box::new(err)),
                                    None,
                                )
                            });
                            buf.advance(abs_pos + 1);
                            self.json_cursor.current_offset = 0;
                            self.json_cursor.delimiter_expected = false;
                            return result;
                        }
                    }
                }
                b']' if !self.json_cursor.quote_opened && self.json_cursor.opened_brackets > 0 => {
                    self.json_cursor.opened_brackets -= 1;
                    self.json_cursor.escaped = false;
                    if self.json_cursor.opened_brackets == 0 {
                        // Closed a nested array/object item
                        self.json_cursor.delimiter_expected = true;
                        let obj_slice = &buf[self.json_cursor.current_obj_pos..abs_pos + 1];
                        let result = serde_json::from_slice(obj_slice).map_err(|err| {
                            StreamBodyError::new(
                                StreamBodyKind::CodecError,
                                Some(Box::new(err)),
                                None,
                            )
                        });
                        self.json_cursor.current_obj_pos = 0;
                        buf.advance(abs_pos + 1);
                        self.json_cursor.current_offset = 0;
                        return result;
                    }
                }
                b'"' if !self.json_cursor.escaped && self.json_cursor.opened_brackets == 0 => {
                    if self.json_cursor.quote_opened {
                        // Closing quote of a top-level string item
                        self.json_cursor.quote_opened = false;
                        if let Some(prim_start) = self.json_cursor.current_primitive_start.take() {
                            self.json_cursor.delimiter_expected = true;
                            let obj_slice = &buf[prim_start..abs_pos + 1];
                            let result = serde_json::from_slice(obj_slice).map_err(|err| {
                                StreamBodyError::new(
                                    StreamBodyKind::CodecError,
                                    Some(Box::new(err)),
                                    None,
                                )
                            });
                            buf.advance(abs_pos + 1);
                            self.json_cursor.current_offset = 0;
                            return result;
                        }
                    } else {
                        // Opening quote of a top-level string item
                        self.json_cursor.quote_opened = true;
                        if self.json_cursor.current_primitive_start.is_none() {
                            self.json_cursor.current_primitive_start = Some(abs_pos);
                        }
                    }
                }
                b'"' if !self.json_cursor.escaped => {
                    // Inside a nested object/array
                    self.json_cursor.quote_opened = !self.json_cursor.quote_opened;
                }
                b'\\' if self.json_cursor.quote_opened => {
                    self.json_cursor.escaped = !self.json_cursor.escaped;
                }
                b'{' if !self.json_cursor.quote_opened => {
                    if self.json_cursor.opened_brackets == 0 {
                        self.json_cursor.current_obj_pos = abs_pos;
                        self.json_cursor.current_primitive_start = None;
                    }
                    self.json_cursor.opened_brackets += 1;
                    self.json_cursor.escaped = false;
                }
                b'}' if !self.json_cursor.quote_opened => {
                    self.json_cursor.opened_brackets -= 1;
                    self.json_cursor.escaped = false;
                    if self.json_cursor.opened_brackets == 0 {
                        self.json_cursor.delimiter_expected = true;
                        let obj_slice = &buf[self.json_cursor.current_obj_pos..abs_pos + 1];
                        let result = serde_json::from_slice(obj_slice).map_err(|err| {
                            StreamBodyError::new(
                                StreamBodyKind::CodecError,
                                Some(Box::new(err)),
                                None,
                            )
                        });
                        self.json_cursor.current_obj_pos = 0;
                        buf.advance(abs_pos + 1);
                        self.json_cursor.current_offset = 0;
                        return result;
                    }
                }
                b',' if !self.json_cursor.quote_opened && self.json_cursor.opened_brackets == 0 => {
                    if let Some(prim_start) = self.json_cursor.current_primitive_start.take() {
                        let obj_slice = trim_ascii(&buf[prim_start..abs_pos]);
                        if !obj_slice.is_empty() {
                            let result = serde_json::from_slice(obj_slice).map_err(|err| {
                                StreamBodyError::new(
                                    StreamBodyKind::CodecError,
                                    Some(Box::new(err)),
                                    None,
                                )
                            });
                            buf.advance(abs_pos + 1);
                            self.json_cursor.current_offset = 0;
                            self.json_cursor.delimiter_expected = false;
                            return result;
                        }
                    } else if !self.json_cursor.delimiter_expected {
                        return Err(StreamBodyError::new(
                            StreamBodyKind::CodecError,
                            None,
                            Some("Unexpected delimiter found".into()),
                        ));
                    }
                    self.json_cursor.delimiter_expected = false;
                }
                _ if !self.json_cursor.quote_opened
                    && self.json_cursor.opened_brackets == 0
                    && self.json_cursor.array_is_opened
                    && !current_ch.is_ascii_whitespace() =>
                {
                    // Non-whitespace character at top level inside array — start of a primitive
                    if self.json_cursor.current_primitive_start.is_none() {
                        self.json_cursor.current_primitive_start = Some(abs_pos);
                    }
                    self.json_cursor.escaped = false;
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

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(bytes.len());
    let end = bytes.iter().rposition(|b| !b.is_ascii_whitespace()).map(|i| i + 1).unwrap_or(0);
    if start >= end { &[] } else { &bytes[start..end] }
}
