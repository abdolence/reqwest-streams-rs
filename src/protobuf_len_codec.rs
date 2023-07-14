use crate::error::StreamBodyKind;
use crate::StreamBodyError;
use bytes::{Buf, BytesMut};
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub struct ProtobufLenPrefixCodec<T> {
    max_length: usize,
    cursor: ProtobufCursor,
    _ph: PhantomData<T>,
}

#[derive(Clone, Debug)]
struct ProtobufCursor {
    current_obj_len: usize,
}

impl<T> ProtobufLenPrefixCodec<T> {
    pub fn new_with_max_length(max_length: usize) -> Self {
        let initial_cursor = ProtobufCursor { current_obj_len: 0 };

        ProtobufLenPrefixCodec {
            max_length,
            cursor: initial_cursor,
            _ph: PhantomData,
        }
    }
}

impl<T> tokio_util::codec::Decoder for ProtobufLenPrefixCodec<T>
where
    T: prost::Message + Default,
{
    type Item = T;
    type Error = StreamBodyError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<T>, StreamBodyError> {
        let buf_len = buf.len();
        if buf_len == 0 {
            return Ok(None);
        }

        if self.cursor.current_obj_len == 0 {
            let bytes = buf.chunk();
            let byte = bytes[0];
            if byte < 0x80 {
                buf.advance(1);
                self.cursor.current_obj_len = u64::from(byte) as usize;
                Ok(None)
            } else if buf_len > 10 || bytes[buf_len - 1] < 0x80 {
                let (value, advance) = decode_varint_slice(bytes)?;
                buf.advance(advance);
                self.cursor.current_obj_len = value as usize;
                Ok(None)
            } else {
                Ok(None) // wait more bytes for len
            }
        } else if self.cursor.current_obj_len > self.max_length {
            Err(StreamBodyError::new(
                StreamBodyKind::MaxLenReachedError,
                None,
                Some("Max object length reached".into()),
            ))
        } else if buf_len >= self.cursor.current_obj_len {
            let obj_bytes = buf.copy_to_bytes(self.cursor.current_obj_len);
            let result: Result<Option<T>, StreamBodyError> = prost::Message::decode(obj_bytes)
                .map(|res| Some(res))
                .map_err(|err| {
                    StreamBodyError::new(StreamBodyKind::CodecError, Some(Box::new(err)), None)
                });
            self.cursor.current_obj_len = 0;
            result
        } else {
            Ok(None)
        }
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<T>, StreamBodyError> {
        self.decode(buf)
    }
}

/// This function is copied from Prost, since it is not available as public API yet optimized for performance.
///
/// Decodes a LEB128-encoded variable length integer from the slice, returning the value and the
/// number of bytes read.
///
/// Based loosely on [`ReadVarint64FromArray`][1] with a varint overflow check from
/// [`ConsumeVarint`][2].
///
/// ## Safety
///
/// The caller must ensure that `bytes` is non-empty and either `bytes.len() >= 10` or the last
/// element in bytes is < `0x80`.
///
/// [1]: https://github.com/google/protobuf/blob/3.3.x/src/google/protobuf/io/coded_stream.cc#L365-L406
/// [2]: https://github.com/protocolbuffers/protobuf-go/blob/v1.27.1/encoding/protowire/wire.go#L358
#[inline]
fn decode_varint_slice(bytes: &[u8]) -> Result<(u64, usize), StreamBodyError> {
    // Fully unrolled varint decoding loop. Splitting into 32-bit pieces gives better performance.

    // Use assertions to ensure memory safety, but it should always be optimized after inline.
    assert!(!bytes.is_empty());
    assert!(bytes.len() > 10 || bytes[bytes.len() - 1] < 0x80);

    let mut b: u8 = bytes[0];
    let mut part0: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((u64::from(part0), 1));
    };
    part0 -= 0x80;
    b = bytes[1];
    part0 += u32::from(b) << 7;
    if b < 0x80 {
        return Ok((u64::from(part0), 2));
    };
    part0 -= 0x80 << 7;
    b = bytes[2];
    part0 += u32::from(b) << 14;
    if b < 0x80 {
        return Ok((u64::from(part0), 3));
    };
    part0 -= 0x80 << 14;
    b = bytes[3];
    part0 += u32::from(b) << 21;
    if b < 0x80 {
        return Ok((u64::from(part0), 4));
    };
    part0 -= 0x80 << 21;
    let value = u64::from(part0);

    b = bytes[4];
    let mut part1: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 5));
    };
    part1 -= 0x80;
    b = bytes[5];
    part1 += u32::from(b) << 7;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 6));
    };
    part1 -= 0x80 << 7;
    b = bytes[6];
    part1 += u32::from(b) << 14;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 7));
    };
    part1 -= 0x80 << 14;
    b = bytes[7];
    part1 += u32::from(b) << 21;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 8));
    };
    part1 -= 0x80 << 21;
    let value = value + ((u64::from(part1)) << 28);

    b = bytes[8];
    let mut part2: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((value + (u64::from(part2) << 56), 9));
    };
    part2 -= 0x80;
    b = bytes[9];
    part2 += u32::from(b) << 7;
    // Check for u64::MAX overflow. See [`ConsumeVarint`][1] for details.
    // [1]: https://github.com/protocolbuffers/protobuf-go/blob/v1.27.1/encoding/protowire/wire.go#L358
    if b < 0x02 {
        return Ok((value + (u64::from(part2) << 56), 10));
    };

    // We have overrun the maximum size of a varint (10 bytes) or the final byte caused an overflow.
    // Assume the data is corrupt.
    Err(StreamBodyError::new(
        StreamBodyKind::CodecError,
        None,
        Some("invalid varint".into()),
    ))
}
