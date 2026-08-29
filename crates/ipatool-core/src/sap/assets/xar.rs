//! Minimal XAR reader.
//!
//! Only enough of the format to locate one entry's raw bytes in the archive
//! heap: a 28-byte header, a zlib-compressed XML table of contents, then the
//! heap itself. Entries are read by HTTP range request rather than downloaded,
//! so nothing here streams the archive.

use std::io::Read;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::error::ClientError;

const MAGIC: &[u8; 4] = b"xar!";
pub const HEADER_LEN: usize = 28;

/// A table-of-contents entry, with `offset` relative to the start of the heap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Header {
    pub size: u64,
    pub toc_compressed: u64,
}

impl Header {
    /// Absolute offset of the archive heap.
    pub fn heap(&self) -> u64 {
        self.size + self.toc_compressed
    }
}

pub fn parse_header(data: &[u8]) -> Result<Header, ClientError> {
    if data.len() < HEADER_LEN {
        return Err(ClientError::Sap(format!(
            "XAR header is {} bytes, expected at least {HEADER_LEN}",
            data.len()
        )));
    }

    if &data[0..4] != MAGIC {
        return Err(ClientError::Sap("not a XAR archive".into()));
    }

    let size = u16::from_be_bytes([data[4], data[5]]) as u64;
    let toc_compressed = u64::from_be_bytes(data[8..16].try_into().unwrap());

    if size < HEADER_LEN as u64 {
        return Err(ClientError::Sap(format!(
            "XAR header size {size} is too small"
        )));
    }

    Ok(Header {
        size,
        toc_compressed,
    })
}

pub fn inflate_toc(compressed: &[u8]) -> Result<Vec<u8>, ClientError> {
    let mut toc = Vec::new();
    flate2::read::ZlibDecoder::new(compressed)
        .read_to_end(&mut toc)
        .map_err(|e| ClientError::Sap(format!("decompress XAR table of contents: {e}")))?;

    Ok(toc)
}

/// Finds a top-level-or-nested entry by name. Entries nest, and only the
/// innermost `<data>` belongs to the file that encloses it, so this tracks a
/// stack rather than scanning the document flat.
pub fn find_entry(toc: &[u8], wanted: &str) -> Result<Entry, ClientError> {
    #[derive(Default)]
    struct Pending {
        name: Option<String>,
        offset: Option<u64>,
        length: Option<u64>,
    }

    let mut reader = Reader::from_reader(toc);
    let mut stack: Vec<Pending> = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut buffer = Vec::new();

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|e| ClientError::Sap(format!("parse XAR table of contents: {e}")))?;

        match event {
            Event::Start(tag) => {
                let name = String::from_utf8_lossy(tag.name().as_ref()).into_owned();
                if name == "file" {
                    stack.push(Pending::default());
                }
                path.push(name);
            }
            Event::End(tag) => {
                let name = String::from_utf8_lossy(tag.name().as_ref()).into_owned();
                path.pop();

                if name == "file"
                    && let Some(pending) = stack.pop()
                    && pending.name.as_deref() == Some(wanted)
                    && let (Some(offset), Some(length)) = (pending.offset, pending.length)
                {
                    return Ok(Entry {
                        name: wanted.to_string(),
                        offset,
                        length,
                    });
                }
            }
            Event::Text(text) => {
                let Some(pending) = stack.last_mut() else {
                    continue;
                };

                let value = text
                    .unescape()
                    .map_err(|e| ClientError::Sap(format!("decode XAR text: {e}")))?
                    .into_owned();

                match path_suffix(&path).as_slice() {
                    ["file", "name"] => pending.name = Some(value),
                    ["data", "offset"] => pending.offset = value.parse().ok(),
                    ["data", "length"] => pending.length = value.parse().ok(),
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }

        buffer.clear();
    }

    Err(ClientError::Sap(format!(
        "XAR archive has no entry named {wanted}"
    )))
}

fn path_suffix(path: &[String]) -> Vec<&str> {
    path.iter()
        .rev()
        .take(2)
        .rev()
        .map(String::as_str)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header() {
        let mut data = vec![0u8; HEADER_LEN];
        data[0..4].copy_from_slice(MAGIC);
        data[4..6].copy_from_slice(&28u16.to_be_bytes());
        data[8..16].copy_from_slice(&4259u64.to_be_bytes());

        let header = parse_header(&data).unwrap();
        assert_eq!(header.size, 28);
        assert_eq!(header.toc_compressed, 4259);
        assert_eq!(header.heap(), 4287);
    }

    #[test]
    fn rejects_foreign_archive() {
        assert!(parse_header(&[0u8; HEADER_LEN]).is_err());
    }

    /// Mirrors the shape of the real update package: a nested `Payload` whose
    /// sibling entries carry their own `<data>` blocks.
    #[test]
    fn finds_nested_entry_without_crossing_siblings() {
        let toc = br#"<xar><toc>
            <file id="1">
                <name>Distribution</name>
                <data><offset>10</offset><length>20</length></data>
            </file>
            <file id="2">
                <name>package.pkg</name>
                <file id="3">
                    <name>Payload</name>
                    <data><offset>4175029</offset><length>1271969659</length></data>
                </file>
                <file id="4">
                    <name>Bom</name>
                    <data><offset>9492</offset><length>4123602</length></data>
                </file>
            </file>
        </toc></xar>"#;

        assert_eq!(
            find_entry(toc, "Payload").unwrap(),
            Entry {
                name: "Payload".to_string(),
                offset: 4175029,
                length: 1271969659,
            }
        );
    }

    #[test]
    fn reports_missing_entry() {
        let toc = br#"<xar><toc><file><name>Bom</name>
            <data><offset>1</offset><length>2</length></data></file></toc></xar>"#;

        assert!(find_entry(toc, "Payload").is_err());
    }
}
