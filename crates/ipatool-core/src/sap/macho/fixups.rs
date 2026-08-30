//! Decoding of dyld's rebase and bind opcode streams.
//!
//! These binaries predate chained fixups, so relocations arrive as the classic
//! `LC_DYLD_INFO_ONLY` bytecode: a tiny stack machine whose state is a segment
//! index, an offset within it, a type, an addend and a symbol name, and whose
//! `DO_*` opcodes emit fixups at the current position.

use crate::error::ClientError;

// Opcode groups share the high nibble; the low nibble is an immediate.
const OPCODE_MASK: u8 = 0xF0;
const IMMEDIATE_MASK: u8 = 0x0F;

const REBASE_DONE: u8 = 0x00;
const REBASE_SET_TYPE_IMM: u8 = 0x10;
const REBASE_SET_SEGMENT_AND_OFFSET_ULEB: u8 = 0x20;
const REBASE_ADD_ADDR_ULEB: u8 = 0x30;
const REBASE_ADD_ADDR_IMM_SCALED: u8 = 0x40;
const REBASE_DO_REBASE_IMM_TIMES: u8 = 0x50;
const REBASE_DO_REBASE_ULEB_TIMES: u8 = 0x60;
const REBASE_DO_REBASE_ADD_ADDR_ULEB: u8 = 0x70;
const REBASE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB: u8 = 0x80;

const BIND_DONE: u8 = 0x00;
const BIND_SET_DYLIB_ORDINAL_IMM: u8 = 0x10;
const BIND_SET_DYLIB_ORDINAL_ULEB: u8 = 0x20;
const BIND_SET_DYLIB_SPECIAL_IMM: u8 = 0x30;
const BIND_SET_SYMBOL_TRAILING_FLAGS_IMM: u8 = 0x40;
const BIND_SET_TYPE_IMM: u8 = 0x50;
const BIND_SET_ADDEND_SLEB: u8 = 0x60;
const BIND_SET_SEGMENT_AND_OFFSET_ULEB: u8 = 0x70;
const BIND_ADD_ADDR_ULEB: u8 = 0x80;
const BIND_DO_BIND: u8 = 0x90;
const BIND_DO_BIND_ADD_ADDR_ULEB: u8 = 0xA0;
const BIND_DO_BIND_ADD_ADDR_IMM_SCALED: u8 = 0xB0;
const BIND_DO_BIND_ULEB_TIMES_SKIPPING_ULEB: u8 = 0xC0;

/// The only relocation kind these images use.
const TYPE_POINTER: u8 = 1;

const POINTER_SIZE: u64 = 8;

/// Bounds the work a malformed stream can ask for.
const MAX_FIXUPS: usize = 4 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rebase {
    pub segment: usize,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    pub segment: usize,
    pub offset: u64,
    pub name: String,
    pub addend: i64,
}

pub fn parse_rebases(stream: &[u8]) -> Result<Vec<Rebase>, ClientError> {
    let mut cursor = Cursor::new(stream);
    let mut out = Vec::new();

    let mut segment = 0usize;
    let mut offset = 0u64;
    // dyld defaults to a pointer rebase when the stream omits SET_TYPE.
    let mut kind = TYPE_POINTER;

    while let Some(byte) = cursor.byte() {
        let opcode = byte & OPCODE_MASK;
        let immediate = byte & IMMEDIATE_MASK;

        match opcode {
            REBASE_DONE => break,
            REBASE_SET_TYPE_IMM => kind = immediate,
            REBASE_SET_SEGMENT_AND_OFFSET_ULEB => {
                segment = immediate as usize;
                offset = cursor.uleb()?;
            }
            REBASE_ADD_ADDR_ULEB => offset = advance(offset, cursor.uleb()?),
            REBASE_ADD_ADDR_IMM_SCALED => {
                offset = advance(offset, u64::from(immediate) * POINTER_SIZE);
            }
            REBASE_DO_REBASE_IMM_TIMES => {
                emit_rebases(
                    &mut out,
                    segment,
                    &mut offset,
                    u64::from(immediate),
                    0,
                    kind,
                )?;
            }
            REBASE_DO_REBASE_ULEB_TIMES => {
                let count = cursor.uleb()?;
                emit_rebases(&mut out, segment, &mut offset, count, 0, kind)?;
            }
            REBASE_DO_REBASE_ADD_ADDR_ULEB => {
                let skip = cursor.uleb()?;
                emit_rebases(&mut out, segment, &mut offset, 1, skip, kind)?;
            }
            REBASE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB => {
                let count = cursor.uleb()?;
                let skip = cursor.uleb()?;
                emit_rebases(&mut out, segment, &mut offset, count, skip, kind)?;
            }
            _ => {
                return Err(ClientError::Sap(format!(
                    "unsupported rebase opcode {opcode:#04x}"
                )));
            }
        }
    }

    Ok(out)
}

pub fn parse_binds(stream: &[u8]) -> Result<Vec<Bind>, ClientError> {
    let mut cursor = Cursor::new(stream);
    let mut out = Vec::new();

    let mut segment = 0usize;
    let mut offset = 0u64;
    let mut addend = 0i64;
    let mut name = String::new();
    // Lazy bind streams routinely omit SET_TYPE; dyld's default is a pointer.
    let mut kind = TYPE_POINTER;

    while let Some(byte) = cursor.byte() {
        let opcode = byte & OPCODE_MASK;
        let immediate = byte & IMMEDIATE_MASK;

        match opcode {
            // A lazy bind stream is a concatenation of per-symbol programs,
            // each terminated by DONE, so this cannot stop the walk.
            BIND_DONE => {}
            BIND_SET_DYLIB_ORDINAL_IMM | BIND_SET_DYLIB_SPECIAL_IMM => {}
            BIND_SET_DYLIB_ORDINAL_ULEB => {
                cursor.uleb()?;
            }
            BIND_SET_SYMBOL_TRAILING_FLAGS_IMM => name = cursor.string()?,
            BIND_SET_TYPE_IMM => kind = immediate,
            BIND_SET_ADDEND_SLEB => addend = cursor.sleb()?,
            BIND_SET_SEGMENT_AND_OFFSET_ULEB => {
                segment = immediate as usize;
                offset = cursor.uleb()?;
            }
            BIND_ADD_ADDR_ULEB => offset = advance(offset, cursor.uleb()?),
            BIND_DO_BIND => {
                emit_binds(&mut out, segment, &mut offset, &name, addend, 1, 0, kind)?;
            }
            BIND_DO_BIND_ADD_ADDR_ULEB => {
                let skip = cursor.uleb()?;
                emit_binds(&mut out, segment, &mut offset, &name, addend, 1, skip, kind)?;
            }
            BIND_DO_BIND_ADD_ADDR_IMM_SCALED => {
                let skip = u64::from(immediate) * POINTER_SIZE;
                emit_binds(&mut out, segment, &mut offset, &name, addend, 1, skip, kind)?;
            }
            BIND_DO_BIND_ULEB_TIMES_SKIPPING_ULEB => {
                let count = cursor.uleb()?;
                let skip = cursor.uleb()?;
                emit_binds(
                    &mut out,
                    segment,
                    &mut offset,
                    &name,
                    addend,
                    count,
                    skip,
                    kind,
                )?;
            }
            _ => {
                return Err(ClientError::Sap(format!(
                    "unsupported bind opcode {opcode:#04x}"
                )));
            }
        }
    }

    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn emit_binds(
    out: &mut Vec<Bind>,
    segment: usize,
    offset: &mut u64,
    name: &str,
    addend: i64,
    count: u64,
    skip: u64,
    kind: u8,
) -> Result<(), ClientError> {
    check_type(kind, "bind")?;

    if name.is_empty() {
        return Err(ClientError::Sap("bind without a symbol name".into()));
    }

    for _ in 0..count {
        push(
            out,
            Bind {
                segment,
                offset: *offset,
                name: name.to_string(),
                addend,
            },
        )?;

        *offset = advance(*offset, POINTER_SIZE.wrapping_add(skip));
    }

    Ok(())
}

fn emit_rebases(
    out: &mut Vec<Rebase>,
    segment: usize,
    offset: &mut u64,
    count: u64,
    skip: u64,
    kind: u8,
) -> Result<(), ClientError> {
    check_type(kind, "rebase")?;

    for _ in 0..count {
        push(
            out,
            Rebase {
                segment,
                offset: *offset,
            },
        )?;

        *offset = advance(*offset, POINTER_SIZE.wrapping_add(skip));
    }

    Ok(())
}

fn push<T>(out: &mut Vec<T>, item: T) -> Result<(), ClientError> {
    if out.len() >= MAX_FIXUPS {
        return Err(ClientError::Sap(format!(
            "image declares more than {MAX_FIXUPS} fixups"
        )));
    }

    out.push(item);

    Ok(())
}

fn check_type(kind: u8, label: &str) -> Result<(), ClientError> {
    if kind != TYPE_POINTER {
        return Err(ClientError::Sap(format!("unsupported {label} type {kind}")));
    }

    Ok(())
}

/// Advances a segment offset.
///
/// dyld encodes backwards moves as huge ULEB values that are meant to wrap —
/// every one of these images has `ADD_ADDR_ULEB 0xfffffffffffff6c8` and similar
/// in its bind stream — so this deliberately wraps rather than rejecting them.
/// The resulting offset is bounds-checked against the segment when the fixup is
/// applied, which is where it actually matters.
fn advance(offset: u64, delta: u64) -> u64 {
    offset.wrapping_add(delta)
}

struct Cursor<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    fn byte(&mut self) -> Option<u8> {
        let byte = *self.data.get(self.position)?;
        self.position += 1;

        Some(byte)
    }

    fn uleb(&mut self) -> Result<u64, ClientError> {
        let mut result = 0u64;
        let mut shift = 0u32;

        loop {
            let byte = self
                .byte()
                .ok_or_else(|| ClientError::Sap("truncated ULEB128".into()))?;

            if shift >= 64 {
                return Err(ClientError::Sap("ULEB128 exceeds 64 bits".into()));
            }

            result |= u64::from(byte & 0x7F) << shift;
            shift += 7;

            if byte & 0x80 == 0 {
                return Ok(result);
            }
        }
    }

    fn sleb(&mut self) -> Result<i64, ClientError> {
        let mut result = 0i64;
        let mut shift = 0u32;

        loop {
            let byte = self
                .byte()
                .ok_or_else(|| ClientError::Sap("truncated SLEB128".into()))?;

            if shift >= 64 {
                return Err(ClientError::Sap("SLEB128 exceeds 64 bits".into()));
            }

            result |= i64::from(byte & 0x7F) << shift;
            shift += 7;

            if byte & 0x80 == 0 {
                // Sign-extend when the payload's high bit is set.
                if shift < 64 && byte & 0x40 != 0 {
                    result |= -1i64 << shift;
                }

                return Ok(result);
            }
        }
    }

    fn string(&mut self) -> Result<String, ClientError> {
        let start = self.position;

        while let Some(byte) = self.byte() {
            if byte == 0 {
                return Ok(
                    String::from_utf8_lossy(&self.data[start..self.position - 1]).into_owned(),
                );
            }
        }

        Err(ClientError::Sap("unterminated symbol name".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_unsigned_and_signed_leb128() {
        assert_eq!(Cursor::new(&[0x00]).uleb().unwrap(), 0);
        assert_eq!(Cursor::new(&[0x7F]).uleb().unwrap(), 127);
        assert_eq!(Cursor::new(&[0x80, 0x01]).uleb().unwrap(), 128);
        assert_eq!(Cursor::new(&[0xE5, 0x8E, 0x26]).uleb().unwrap(), 624_485);

        assert_eq!(Cursor::new(&[0x00]).sleb().unwrap(), 0);
        assert_eq!(Cursor::new(&[0x7F]).sleb().unwrap(), -1);
        assert_eq!(Cursor::new(&[0x3F]).sleb().unwrap(), 63);
        assert_eq!(Cursor::new(&[0x40]).sleb().unwrap(), -64);
        assert_eq!(Cursor::new(&[0x80, 0x7F]).sleb().unwrap(), -128);
    }

    #[test]
    fn rejects_truncated_leb128() {
        assert!(Cursor::new(&[0x80]).uleb().is_err());
        assert!(Cursor::new(&[0x80]).sleb().is_err());
    }

    #[test]
    fn rebases_step_by_pointer_size() {
        // SET_SEGMENT_AND_OFFSET(seg 2, off 0x10); DO_REBASE_IMM_TIMES(3); DONE
        let stream = [
            REBASE_SET_SEGMENT_AND_OFFSET_ULEB | 2,
            0x10,
            REBASE_DO_REBASE_IMM_TIMES | 3,
            REBASE_DONE,
        ];

        assert_eq!(
            parse_rebases(&stream).unwrap(),
            vec![
                Rebase {
                    segment: 2,
                    offset: 0x10
                },
                Rebase {
                    segment: 2,
                    offset: 0x18
                },
                Rebase {
                    segment: 2,
                    offset: 0x20
                },
            ]
        );
    }

    #[test]
    fn rebase_skip_is_added_to_the_pointer_stride() {
        // TIMES_SKIPPING(count 2, skip 8) => stride of 16
        let stream = [
            REBASE_SET_SEGMENT_AND_OFFSET_ULEB | 1,
            0x00,
            REBASE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB,
            0x02,
            0x08,
            REBASE_DONE,
        ];

        let offsets: Vec<u64> = parse_rebases(&stream)
            .unwrap()
            .iter()
            .map(|r| r.offset)
            .collect();

        assert_eq!(offsets, vec![0x00, 0x10]);
    }

    #[test]
    fn binds_carry_symbol_and_addend() {
        let mut stream = vec![BIND_SET_SEGMENT_AND_OFFSET_ULEB | 1, 0x20];
        stream.push(BIND_SET_SYMBOL_TRAILING_FLAGS_IMM);
        stream.extend_from_slice(b"_malloc\0");
        stream.push(BIND_SET_ADDEND_SLEB);
        stream.push(0x7F); // -1
        stream.push(BIND_DO_BIND);
        stream.push(BIND_DONE);

        assert_eq!(
            parse_binds(&stream).unwrap(),
            vec![Bind {
                segment: 1,
                offset: 0x20,
                name: "_malloc".to_string(),
                addend: -1,
            }]
        );
    }

    /// A lazy bind stream is many small programs concatenated, each ending in
    /// DONE. Stopping at the first one would silently drop every later symbol.
    #[test]
    fn bind_done_does_not_end_the_stream() {
        let mut stream = Vec::new();

        for symbol in ["_first", "_second"] {
            stream.push(BIND_SET_SEGMENT_AND_OFFSET_ULEB | 2);
            stream.push(0x00);
            stream.push(BIND_SET_SYMBOL_TRAILING_FLAGS_IMM);
            stream.extend_from_slice(symbol.as_bytes());
            stream.push(0);
            stream.push(BIND_DO_BIND);
            stream.push(BIND_DONE);
        }

        let names: Vec<String> = parse_binds(&stream)
            .unwrap()
            .into_iter()
            .map(|b| b.name)
            .collect();

        assert_eq!(names, vec!["_first", "_second"]);
    }

    #[test]
    fn rejects_a_bind_without_a_symbol() {
        let stream = [BIND_SET_SEGMENT_AND_OFFSET_ULEB, 0x00, BIND_DO_BIND];

        assert!(parse_binds(&stream).is_err());
    }

    #[test]
    fn rejects_non_pointer_fixups() {
        // Text relocations would need the image mapped writable and executable.
        let stream = [
            REBASE_SET_TYPE_IMM | 2,
            REBASE_SET_SEGMENT_AND_OFFSET_ULEB,
            0x00,
            REBASE_DO_REBASE_IMM_TIMES | 1,
        ];

        assert!(parse_rebases(&stream).is_err());
    }

    /// Every one of Apple's frameworks encodes backwards moves as a ULEB that
    /// is meant to wrap; rejecting the overflow rejects the real binaries.
    #[test]
    fn add_addr_wraps_for_negative_deltas() {
        fn uleb(mut value: u64, out: &mut Vec<u8>) {
            loop {
                let byte = (value & 0x7F) as u8;
                value >>= 7;

                if value == 0 {
                    out.push(byte);

                    return;
                }

                out.push(byte | 0x80);
            }
        }

        let start = 0x9d8u64;
        let delta = 0u64.wrapping_sub(0x938);

        let mut stream = vec![BIND_SET_SEGMENT_AND_OFFSET_ULEB | 1];
        uleb(start, &mut stream);
        stream.push(BIND_SET_SYMBOL_TRAILING_FLAGS_IMM);
        stream.extend_from_slice(b"_Gestalt\0");
        stream.push(BIND_ADD_ADDR_ULEB);
        uleb(delta, &mut stream);
        stream.push(BIND_DO_BIND);

        let binds = parse_binds(&stream).unwrap();

        assert_eq!(binds.len(), 1);
        assert_eq!(binds[0].offset, 0xa0);
    }
}
