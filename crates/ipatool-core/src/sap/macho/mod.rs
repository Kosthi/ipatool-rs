//! Loading x86-64 Mach-O images into guest memory.
//!
//! This is deliberately not a dyld: nothing is executed at load time, no
//! dependent libraries are followed, and no initializers run. The images are
//! placed at a chosen base, their pointer fixups are applied, and every import
//! is resolved by the caller — to a real export in a sibling image, or to a
//! shim stub.

pub mod fixups;

use std::collections::HashMap;

use crate::error::ClientError;

const FAT_MAGIC: u32 = 0xCAFE_BABE;
const MH_MAGIC_64: u32 = 0xFEED_FACF;
const CPU_TYPE_X86_64: u32 = 0x0100_0007;

const LC_SEGMENT_64: u32 = 0x19;
const LC_SYMTAB: u32 = 0x02;
const LC_DYLD_INFO: u32 = 0x22;
const LC_DYLD_INFO_ONLY: u32 = 0x8000_0022;

const HEADER_64_SIZE: usize = 32;
const SEGMENT_64_SIZE: usize = 72;
const NLIST_64_SIZE: usize = 16;

const PAGE_SIZE: u64 = 0x1000;
const POINTER_SIZE: u64 = 8;

/// An image spanning more than this is not one of Apple's frameworks.
const MAX_IMAGE_SPAN: u64 = 1 << 30;

/// Somewhere for a loaded image to write bytes.
pub trait Memory {
    fn map(&self, address: u64, size: u64) -> Result<(), ClientError>;
    fn write(&self, address: u64, data: &[u8]) -> Result<(), ClientError>;
}

#[derive(Debug, Clone)]
struct Segment {
    name: String,
    address: u64,
    size: u64,
    file_offset: u64,
    file_size: u64,
}

pub struct Image {
    name: String,
    /// A private copy: fixups are applied here before it reaches the guest.
    data: Vec<u8>,
    segments: Vec<Segment>,
    symbols: HashMap<String, u64>,
    base: u64,
    rebases: Vec<fixups::Rebase>,
    binds: Vec<fixups::Bind>,
    loaded_base: Option<u64>,
}

impl Image {
    pub fn open(name: &str, input: &[u8]) -> Result<Self, ClientError> {
        let data = x86_64_slice(input)
            .map_err(|e| ClientError::Sap(format!("open {name}: {e}")))?
            .to_vec();

        let mut image = Self {
            name: name.to_string(),
            data,
            segments: Vec::new(),
            symbols: HashMap::new(),
            base: 0,
            rebases: Vec::new(),
            binds: Vec::new(),
            loaded_base: None,
        };

        image.parse().map_err(|e| match e {
            ClientError::Sap(message) => ClientError::Sap(format!("{name}: {message}")),
            other => other,
        })?;

        Ok(image)
    }

    fn parse(&mut self) -> Result<(), ClientError> {
        let header = self.data.get(..HEADER_64_SIZE).ok_or_else(short)?;

        if read_u32(header, 0)? != MH_MAGIC_64 {
            return Err(ClientError::Sap("not a 64-bit Mach-O".into()));
        }

        if read_u32(header, 4)? != CPU_TYPE_X86_64 {
            return Err(ClientError::Sap("not an x86-64 Mach-O".into()));
        }

        let command_count = read_u32(header, 16)?;
        let mut position = HEADER_64_SIZE;

        // Collected while walking the commands, applied afterwards so the
        // segment table is complete before offsets are resolved against it.
        let mut symtab = None;
        let mut dyld_info = None;

        for _ in 0..command_count {
            let command = self.data.get(position..position + 8).ok_or_else(short)?;
            let kind = read_u32(command, 0)?;
            let size = read_u32(command, 4)? as usize;

            if size < 8 || position + size > self.data.len() {
                return Err(ClientError::Sap("load command exceeds image".into()));
            }

            let body = &self.data[position..position + size];

            match kind {
                LC_SEGMENT_64 => self.segments.push(parse_segment(body)?),
                LC_SYMTAB => {
                    symtab = Some((
                        read_u32(body, 8)?,
                        read_u32(body, 12)?,
                        read_u32(body, 16)?,
                        read_u32(body, 20)?,
                    ))
                }
                LC_DYLD_INFO | LC_DYLD_INFO_ONLY => {
                    dyld_info = Some(DyldInfo {
                        rebase: (read_u32(body, 8)?, read_u32(body, 12)?),
                        bind: (read_u32(body, 16)?, read_u32(body, 20)?),
                        weak_bind: (read_u32(body, 24)?, read_u32(body, 28)?),
                        lazy_bind: (read_u32(body, 32)?, read_u32(body, 36)?),
                    });
                }
                _ => {}
            }

            position += size;
        }

        self.validate_segments()?;

        // dyld treats __TEXT's address as the image's preferred base; every
        // symbol and fixup target is expressed relative to it.
        self.base = self
            .segments
            .iter()
            .find(|segment| segment.name == "__TEXT")
            .ok_or_else(|| ClientError::Sap("image has no __TEXT segment".into()))?
            .address;

        if let Some((symbol_offset, symbol_count, string_offset, string_size)) = symtab {
            self.parse_symbols(symbol_offset, symbol_count, string_offset, string_size)?;
        }

        if let Some(info) = dyld_info {
            self.rebases = fixups::parse_rebases(self.slice(info.rebase)?)?;

            // All three bind streams are applied eagerly. Lazy binds especially:
            // nothing resolves them later, because there is no dyld here to
            // service a stub.
            for stream in [info.bind, info.weak_bind, info.lazy_bind] {
                self.binds.extend(fixups::parse_binds(self.slice(stream)?)?);
            }
        }

        Ok(())
    }

    fn slice(&self, (offset, size): (u32, u32)) -> Result<&[u8], ClientError> {
        if size == 0 {
            return Ok(&[]);
        }

        let start = offset as usize;
        let end = start
            .checked_add(size as usize)
            .ok_or_else(|| ClientError::Sap("dyld info range overflows".into()))?;

        self.data
            .get(start..end)
            .ok_or_else(|| ClientError::Sap("dyld info range exceeds image".into()))
    }

    fn parse_symbols(
        &mut self,
        symbol_offset: u32,
        symbol_count: u32,
        string_offset: u32,
        string_size: u32,
    ) -> Result<(), ClientError> {
        let strings_start = string_offset as usize;
        let strings_end = strings_start
            .checked_add(string_size as usize)
            .ok_or_else(|| ClientError::Sap("string table overflows".into()))?;

        let strings = self
            .data
            .get(strings_start..strings_end)
            .ok_or_else(|| ClientError::Sap("string table exceeds image".into()))?;

        for index in 0..symbol_count as usize {
            let start = symbol_offset as usize + index * NLIST_64_SIZE;
            let Some(entry) = self.data.get(start..start + NLIST_64_SIZE) else {
                return Err(ClientError::Sap("symbol table exceeds image".into()));
            };

            let name_offset = read_u32(entry, 0)? as usize;
            let kind = entry[4];
            let value = read_u64(entry, 8)?;

            // Only defined symbols in a section (N_SECT) have a usable address;
            // undefined and debug entries do not.
            const N_TYPE: u8 = 0x0E;
            const N_SECT: u8 = 0x0E;
            const N_STAB: u8 = 0xE0;

            if kind & N_STAB != 0 || kind & N_TYPE != N_SECT || value == 0 {
                continue;
            }

            let Some(tail) = strings.get(name_offset..) else {
                continue;
            };

            let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
            if end == 0 {
                continue;
            }

            self.symbols
                .insert(String::from_utf8_lossy(&tail[..end]).into_owned(), value);
        }

        Ok(())
    }

    fn validate_segments(&self) -> Result<(), ClientError> {
        for segment in &self.segments {
            if segment.file_size > segment.size {
                return Err(ClientError::Sap(format!(
                    "segment {} has more file data than memory",
                    segment.name
                )));
            }

            let end = segment
                .file_offset
                .checked_add(segment.file_size)
                .ok_or_else(|| ClientError::Sap("segment file range overflows".into()))?;

            if end > self.data.len() as u64 {
                return Err(ClientError::Sap(format!(
                    "segment {} data exceeds image",
                    segment.name
                )));
            }

            segment
                .address
                .checked_add(segment.size)
                .ok_or_else(|| ClientError::Sap("segment address range overflows".into()))?;
        }

        Ok(())
    }

    /// Address of `name` once the image is placed at `load_base`.
    pub fn export(&self, name: &str, load_base: u64) -> Result<u64, ClientError> {
        let address = *self
            .symbols
            .get(name)
            .ok_or_else(|| ClientError::Sap(format!("{} does not export {name}", self.name)))?;

        if address < self.base {
            return Err(ClientError::Sap(format!(
                "{name} in {} precedes the image base",
                self.name
            )));
        }

        load_base
            .checked_add(address - self.base)
            .ok_or_else(|| ClientError::Sap(format!("{name} address overflows in {}", self.name)))
    }

    /// Applies every pointer fixup for a placement at `load_base`.
    ///
    /// `resolve` supplies the address for each imported symbol.
    pub fn relocate(
        &mut self,
        load_base: u64,
        mut resolve: impl FnMut(&str) -> Result<u64, ClientError>,
    ) -> Result<(), ClientError> {
        if self.loaded_base.is_some() {
            return Err(ClientError::Sap(format!(
                "{} is already relocated",
                self.name
            )));
        }

        let base = self.base;

        for index in 0..self.rebases.len() {
            let rebase = self.rebases[index];
            let offset = self.fixup_offset(rebase.segment, rebase.offset)?;

            // A rebase adjusts the pointer already stored at the location.
            let stored = read_u64(&self.data, offset as usize)?;
            if stored < base {
                return Err(ClientError::Sap(format!(
                    "{} contains a rebase below its image base",
                    self.name
                )));
            }

            let value = load_base.checked_add(stored - base).ok_or_else(|| {
                ClientError::Sap(format!("rebase address overflows in {}", self.name))
            })?;

            self.put_pointer(offset, value)?;
        }

        for index in 0..self.binds.len() {
            let (segment, bind_offset, name, addend) = {
                let bind = &self.binds[index];
                (bind.segment, bind.offset, bind.name.clone(), bind.addend)
            };

            let offset = self.fixup_offset(segment, bind_offset)?;
            let address = resolve(&name)
                .map_err(|e| ClientError::Sap(format!("resolve {name} for {}: {e}", self.name)))?;

            let value = address.checked_add_signed(addend).ok_or_else(|| {
                ClientError::Sap(format!("addend for {name} overflows in {}", self.name))
            })?;

            self.put_pointer(offset, value)?;
        }

        self.loaded_base = Some(load_base);

        Ok(())
    }

    /// Maps the image and writes its segments into guest memory.
    pub fn load(&self, memory: &impl Memory) -> Result<(), ClientError> {
        let loaded_base = self.loaded_base.ok_or_else(|| {
            ClientError::Sap(format!("{} must be relocated before loading", self.name))
        })?;

        let mut span = 0u64;

        for segment in self.loadable() {
            if segment.address < self.base {
                return Err(ClientError::Sap(format!(
                    "segment {} in {} precedes the image base",
                    segment.name, self.name
                )));
            }

            let end = (segment.address - self.base)
                .checked_add(segment.size)
                .filter(|end| *end <= MAX_IMAGE_SPAN)
                .ok_or_else(|| {
                    ClientError::Sap(format!(
                        "segment {} makes {} too large",
                        segment.name, self.name
                    ))
                })?;

            span = span.max(end);
        }

        span = span.next_multiple_of(PAGE_SIZE);

        if span == 0 {
            return Err(ClientError::Sap(format!(
                "{} has no loadable segments",
                self.name
            )));
        }

        memory.map(loaded_base, span)?;

        for segment in self.loadable() {
            if segment.file_size == 0 {
                continue;
            }

            let start = segment.file_offset as usize;
            let end = start + segment.file_size as usize;

            memory.write(
                loaded_base + (segment.address - self.base),
                &self.data[start..end],
            )?;
        }

        Ok(())
    }

    fn loadable(&self) -> impl Iterator<Item = &Segment> {
        self.segments
            .iter()
            .filter(|segment| segment.name != "__PAGEZERO" && segment.size != 0)
    }

    /// Translates a segment-relative fixup location to a file offset.
    fn fixup_offset(&self, segment: usize, offset: u64) -> Result<u64, ClientError> {
        let segment = self.segments.get(segment).ok_or_else(|| {
            ClientError::Sap(format!("fixup names segment {segment} in {}", self.name))
        })?;

        let end = offset
            .checked_add(POINTER_SIZE)
            .ok_or_else(|| ClientError::Sap("fixup offset overflows".into()))?;

        // A fixup must land in bytes that exist in the file, not merely in the
        // segment's zero-filled tail.
        if end > segment.file_size {
            return Err(ClientError::Sap(format!(
                "fixup at {offset:#x} exceeds file data for segment {} in {}",
                segment.name, self.name
            )));
        }

        Ok(segment.file_offset + offset)
    }

    fn put_pointer(&mut self, offset: u64, value: u64) -> Result<(), ClientError> {
        let start = offset as usize;
        let slot = self
            .data
            .get_mut(start..start + POINTER_SIZE as usize)
            .ok_or_else(|| {
                ClientError::Sap(format!("fixup at {offset:#x} exceeds {}", self.name))
            })?;

        slot.copy_from_slice(&value.to_le_bytes());

        Ok(())
    }
}

struct DyldInfo {
    rebase: (u32, u32),
    bind: (u32, u32),
    weak_bind: (u32, u32),
    lazy_bind: (u32, u32),
}

fn parse_segment(body: &[u8]) -> Result<Segment, ClientError> {
    if body.len() < SEGMENT_64_SIZE {
        return Err(ClientError::Sap("truncated segment command".into()));
    }

    let raw = &body[8..24];
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());

    Ok(Segment {
        name: String::from_utf8_lossy(&raw[..end]).into_owned(),
        address: read_u64(body, 24)?,
        size: read_u64(body, 32)?,
        file_offset: read_u64(body, 40)?,
        file_size: read_u64(body, 48)?,
    })
}

/// Returns the x86-64 image, unwrapping a universal binary if necessary.
fn x86_64_slice(input: &[u8]) -> Result<&[u8], ClientError> {
    if input.len() < 8 || read_u32_be(input, 0)? != FAT_MAGIC {
        return Ok(input);
    }

    let count = read_u32_be(input, 4)? as usize;

    for index in 0..count {
        let entry = 8 + index * 20;
        let Some(architecture) = input.get(entry..entry + 20) else {
            return Err(ClientError::Sap("fat header exceeds input".into()));
        };

        if read_u32_be(architecture, 0)? != CPU_TYPE_X86_64 {
            continue;
        }

        let offset = read_u32_be(architecture, 8)? as usize;
        let size = read_u32_be(architecture, 12)? as usize;

        return input
            .get(offset..offset.saturating_add(size))
            .ok_or_else(|| ClientError::Sap("x86-64 slice exceeds input".into()));
    }

    Err(ClientError::Sap(
        "universal binary has no x86-64 slice".into(),
    ))
}

fn short() -> ClientError {
    ClientError::Sap("image is truncated".into())
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, ClientError> {
    data.get(offset..offset + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(short)
}

fn read_u32_be(data: &[u8], offset: usize) -> Result<u32, ClientError> {
    data.get(offset..offset + 4)
        .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
        .ok_or_else(short)
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, ClientError> {
    data.get(offset..offset + 8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(short)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_a_thin_image() {
        let input = [MH_MAGIC_64.to_le_bytes().as_slice(), &[0u8; 28]].concat();

        assert_eq!(x86_64_slice(&input).unwrap(), input.as_slice());
    }

    #[test]
    fn picks_the_x86_64_slice_from_a_universal_binary() {
        let mut input = Vec::new();
        input.extend_from_slice(&FAT_MAGIC.to_be_bytes());
        input.extend_from_slice(&2u32.to_be_bytes());

        // i386 first, x86-64 second: order must not matter.
        for (cpu, offset, size) in [(0x0000_0007u32, 48u32, 4u32), (CPU_TYPE_X86_64, 52, 4)] {
            input.extend_from_slice(&cpu.to_be_bytes());
            input.extend_from_slice(&0u32.to_be_bytes());
            input.extend_from_slice(&offset.to_be_bytes());
            input.extend_from_slice(&size.to_be_bytes());
            input.extend_from_slice(&0u32.to_be_bytes());
        }

        input.resize(48, 0);
        input.extend_from_slice(b"i386");
        input.extend_from_slice(b"amd6");

        assert_eq!(x86_64_slice(&input).unwrap(), b"amd6");
    }

    #[test]
    fn reports_a_universal_binary_without_an_x86_64_slice() {
        let mut input = Vec::new();
        input.extend_from_slice(&FAT_MAGIC.to_be_bytes());
        input.extend_from_slice(&1u32.to_be_bytes());
        input.extend_from_slice(&0x0000_0007u32.to_be_bytes());
        input.extend_from_slice(&[0u8; 16]);

        assert!(x86_64_slice(&input).is_err());
    }

    #[test]
    fn rejects_a_slice_that_runs_past_the_input() {
        let mut input = Vec::new();
        input.extend_from_slice(&FAT_MAGIC.to_be_bytes());
        input.extend_from_slice(&1u32.to_be_bytes());
        input.extend_from_slice(&CPU_TYPE_X86_64.to_be_bytes());
        input.extend_from_slice(&0u32.to_be_bytes());
        input.extend_from_slice(&28u32.to_be_bytes());
        input.extend_from_slice(&999u32.to_be_bytes());
        input.extend_from_slice(&0u32.to_be_bytes());

        assert!(x86_64_slice(&input).is_err());
    }

    /// Parses the real Apple frameworks and checks every symbol the SAP runtime
    /// resolves against the addresses `nm` reports for them.
    ///
    /// `cargo test -p ipatool-core -- --ignored live_macho`
    #[tokio::test]
    #[ignore = "downloads the Apple SAP assets"]
    async fn live_macho_resolves_every_entry_point() {
        let client = crate::client::AppleClient::for_tests();

        let cache = std::env::temp_dir().join("ipatool-rs-sap-assets");
        let bundle = crate::sap::assets::load(&client, &cache)
            .await
            .expect("load assets");

        // Addresses from `nm -arch x86_64`, i.e. image-relative.
        type Expectation<'a> = (&'a str, &'a [u8], &'a [(&'a str, u64)]);
        let expected: [Expectation<'_>; 3] = [
            (
                "CoreFP",
                &bundle.core_fp,
                &[
                    ("_WIn9UJ86JKdV4dM", 0x0009_57c0),
                    ("_X46O5IeS", 0x0043_2440),
                    ("_YlCJ3lg", 0x0058_a100),
                    ("_dku592fbFAj", 0x00b5_f860),
                    ("_fdjkDSAFjklaf2s", 0x0042_a3b0),
                    ("_lxpgvVMLd0S7uRl", 0x0011_a090),
                ],
            ),
            (
                "CommerceCore",
                &bundle.commerce_core,
                &[("_get_mac_address", 0x0000_5d37)],
            ),
            (
                "CommerceKit",
                &bundle.commerce_kit,
                &[
                    ("_cp2g1b9ro", 0x000a_40b0),
                    ("_Mib5yocT", 0x0008_8cd0),
                    ("_Fc3vhtJDvr", 0x0012_3af0),
                    ("_IPaI1oem5iL", 0x000b_a0e0),
                    ("_jEHf8Xzsv8K", 0x000a_1250),
                ],
            ),
        ];

        for (name, data, symbols) in expected {
            let image = Image::open(name, data).expect(name);

            for (symbol, address) in symbols {
                assert_eq!(
                    image.export(symbol, 0).unwrap(),
                    *address,
                    "{name}:{symbol}"
                );
            }

            // Placement must shift every export by the same amount.
            let base = 0x0000_1000_0000_0000;
            assert_eq!(
                image.export(symbols[0].0, base).unwrap(),
                base + symbols[0].1
            );
        }
    }

    /// Records what a loaded image asks for, so the placement can be checked
    /// without an emulator.
    #[derive(Default)]
    struct Recorder {
        mapped: std::cell::RefCell<Vec<(u64, u64)>>,
        written: std::cell::RefCell<u64>,
    }

    impl Memory for Recorder {
        fn map(&self, address: u64, size: u64) -> Result<(), ClientError> {
            self.mapped.borrow_mut().push((address, size));

            Ok(())
        }

        fn write(&self, _address: u64, data: &[u8]) -> Result<(), ClientError> {
            *self.written.borrow_mut() += data.len() as u64;

            Ok(())
        }
    }

    /// Applies every fixup in the real images. `export` alone never touches the
    /// opcode streams, so this is what actually exercises the decoder: each
    /// fixup is bounds-checked against its segment's file data on the way
    /// through, and a misdecoded stream lands outside and fails.
    ///
    /// `cargo test -p ipatool-core -- --ignored live_macho`
    #[tokio::test]
    #[ignore = "downloads the Apple SAP assets"]
    async fn live_macho_relocates_and_loads_every_image() {
        let client = crate::client::AppleClient::for_tests();

        let cache = std::env::temp_dir().join("ipatool-rs-sap-assets");
        let bundle = crate::sap::assets::load(&client, &cache)
            .await
            .expect("load assets");

        // The bases upstream places these at.
        let images = [
            ("CoreFP", &bundle.core_fp, 0x0000_1000_0000_0000u64),
            ("CommerceCore", &bundle.commerce_core, 0x0000_1000_4000_0000),
            ("CommerceKit", &bundle.commerce_kit, 0x0000_1000_8000_0000),
        ];

        // Stands in for the shim area: every import resolves somewhere.
        let stub_base = 0x0000_2000_0000_0000u64;
        let mut imports = 0u64;

        for (name, data, base) in images {
            let mut image = Image::open(name, data).expect(name);

            image
                .relocate(base, |_| {
                    imports += 1;

                    Ok(stub_base)
                })
                .unwrap_or_else(|e| panic!("relocate {name}: {e}"));

            let recorder = Recorder::default();
            image
                .load(&recorder)
                .unwrap_or_else(|e| panic!("load {name}: {e}"));

            let mapped = recorder.mapped.borrow();
            assert_eq!(mapped.len(), 1, "{name} should map one span");
            assert_eq!(mapped[0].0, base, "{name} mapped at the wrong base");
            assert!(*recorder.written.borrow() > 0, "{name} wrote nothing");

            // Relocation is one-shot; a second pass would double-apply rebases.
            assert!(image.relocate(base, |_| Ok(0)).is_err());
        }

        assert!(imports > 0, "no imports were resolved");
    }
}
