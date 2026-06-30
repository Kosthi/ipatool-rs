use std::io::Read;

use flate2::read::DeflateDecoder;

use crate::error::ClientError;

const LOCAL_FILE_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const CENTRAL_DIRECTORY_HEADER_SIGNATURE: u32 = 0x0201_4b50;
const END_OF_CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0605_4b50;
const ZIP64_END_OF_CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0606_4b50;
const ZIP64_END_OF_CENTRAL_DIRECTORY_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;

const EOCD_MIN_SIZE: usize = 22;
const ZIP64_EOCD_LOCATOR_SIZE: u64 = 20;
const ZIP64_EOCD_MIN_SIZE: u64 = 56;
const MAX_ZIP_COMMENT_SIZE: u64 = u16::MAX as u64;
const ZIP32_MAX: u64 = u32::MAX as u64;
const ZIP16_MAX: u64 = u16::MAX as u64;
const ZIP64_EXTRA_FIELD_ID: u16 = 0x0001;

pub struct RemoteInfoPlist {
    pub plist: plist::Dictionary,
    pub modified: Option<zip::DateTime>,
}

struct RemoteZip {
    client: reqwest::Client,
    url: String,
    size: u64,
}

struct EndOfCentralDirectory {
    offset: u64,
    entries: u64,
    central_directory_size: u64,
    central_directory_offset: u64,
}

struct CentralDirectory {
    size: u64,
    offset: u64,
}

struct CentralDirectoryEntry {
    compression_method: u16,
    flags: u16,
    compressed_size: u64,
    uncompressed_size: u64,
    local_header_offset: u64,
    modified: Option<zip::DateTime>,
}

impl RemoteZip {
    async fn new(client: &reqwest::Client, url: &str) -> Result<Self, ClientError> {
        let resp = client
            .get(url)
            .header("Accept-Encoding", "identity")
            .header("Range", "bytes=0-0")
            .send()
            .await?;

        if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(zip_error(format!(
                "expected partial content response, got status {}",
                resp.status()
            )));
        }

        let size = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.rsplit('/').next())
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| zip_error("missing Content-Range for size"))?;

        Ok(Self {
            client: client.clone(),
            url: url.to_string(),
            size,
        })
    }

    async fn read_range(&self, start: u64, len: u64) -> Result<Vec<u8>, ClientError> {
        if len == 0 {
            return Ok(Vec::new());
        }

        if start >= self.size {
            return Err(zip_error("range starts past end of file"));
        }

        let end = start
            .checked_add(len - 1)
            .ok_or_else(|| zip_error("range end overflow"))?
            .min(self.size - 1);
        let range = format!("bytes={start}-{end}");

        let resp = self
            .client
            .get(&self.url)
            .header("Accept-Encoding", "identity")
            .header("Range", range)
            .send()
            .await?;

        if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(zip_error(format!(
                "expected partial content response, got status {}",
                resp.status()
            )));
        }

        let bytes = resp.bytes().await?;
        let expected = as_usize(end - start + 1, "range length")?;
        if bytes.len() != expected {
            return Err(zip_error(format!(
                "short range response: expected {expected} bytes, got {}",
                bytes.len()
            )));
        }

        Ok(bytes.to_vec())
    }

    async fn read_central_directory(&self) -> Result<Vec<u8>, ClientError> {
        let tail_len = self.size.min(MAX_ZIP_COMMENT_SIZE + EOCD_MIN_SIZE as u64);
        let tail_start = self.size - tail_len;
        let tail = self.read_range(tail_start, tail_len).await?;
        let eocd = find_eocd_in_tail(&tail, tail_start)?;
        let directory = if eocd.needs_zip64() {
            self.read_zip64_central_directory(&eocd).await?
        } else {
            CentralDirectory {
                size: eocd.central_directory_size,
                offset: eocd.central_directory_offset,
            }
        };

        self.read_range(directory.offset, directory.size).await
    }

    async fn read_zip64_central_directory(
        &self,
        eocd: &EndOfCentralDirectory,
    ) -> Result<CentralDirectory, ClientError> {
        if eocd.offset < ZIP64_EOCD_LOCATOR_SIZE {
            return Err(zip_error("missing ZIP64 end of central directory locator"));
        }

        let locator = self
            .read_range(
                eocd.offset - ZIP64_EOCD_LOCATOR_SIZE,
                ZIP64_EOCD_LOCATOR_SIZE,
            )
            .await?;
        let signature = read_u32_le(&locator, 0, "ZIP64 locator signature")?;
        if signature != ZIP64_END_OF_CENTRAL_DIRECTORY_LOCATOR_SIGNATURE {
            return Err(zip_error("invalid ZIP64 end of central directory locator"));
        }

        let disk_number = read_u32_le(&locator, 4, "ZIP64 locator disk number")?;
        let eocd64_offset = read_u64_le(&locator, 8, "ZIP64 EOCD offset")?;
        let number_of_disks = read_u32_le(&locator, 16, "ZIP64 locator disk count")?;
        if disk_number != 0 || number_of_disks > 1 {
            return Err(zip_error("multi-disk ZIP archives are not supported"));
        }

        let record = self.read_range(eocd64_offset, ZIP64_EOCD_MIN_SIZE).await?;
        let signature = read_u32_le(&record, 0, "ZIP64 EOCD signature")?;
        if signature != ZIP64_END_OF_CENTRAL_DIRECTORY_SIGNATURE {
            return Err(zip_error("invalid ZIP64 end of central directory"));
        }

        let record_size = read_u64_le(&record, 4, "ZIP64 EOCD size")?;
        if record_size < 44 {
            return Err(zip_error("invalid ZIP64 end of central directory size"));
        }

        let disk_number = read_u32_le(&record, 16, "ZIP64 EOCD disk number")?;
        let central_directory_disk = read_u32_le(&record, 20, "ZIP64 EOCD central directory disk")?;
        if disk_number != 0 || central_directory_disk != 0 {
            return Err(zip_error("multi-disk ZIP archives are not supported"));
        }

        Ok(CentralDirectory {
            size: read_u64_le(&record, 40, "ZIP64 EOCD central directory size")?,
            offset: read_u64_le(&record, 48, "ZIP64 EOCD central directory offset")?,
        })
    }
}

impl EndOfCentralDirectory {
    fn needs_zip64(&self) -> bool {
        self.entries == ZIP16_MAX
            || self.central_directory_size == ZIP32_MAX
            || self.central_directory_offset == ZIP32_MAX
    }
}

pub async fn read_remote_info_plist(
    client: &reqwest::Client,
    url: &str,
) -> Result<RemoteInfoPlist, ClientError> {
    let remote = RemoteZip::new(client, url).await?;
    let central_directory = remote.read_central_directory().await?;
    let entry = find_info_plist_entry(&central_directory)?;

    let local_header = remote.read_range(entry.local_header_offset, 30).await?;
    let data_offset = local_file_data_offset(&local_header, entry.local_header_offset)?;
    let compressed = remote
        .read_range(data_offset, entry.compressed_size)
        .await?;
    let plist_bytes = decompress_entry(&entry, &compressed)?;
    let plist = plist::from_bytes(&plist_bytes).map_err(ClientError::PlistDe)?;

    Ok(RemoteInfoPlist {
        plist,
        modified: entry.modified,
    })
}

fn find_eocd_in_tail(tail: &[u8], tail_start: u64) -> Result<EndOfCentralDirectory, ClientError> {
    if tail.len() < EOCD_MIN_SIZE {
        return Err(zip_error(
            "file is too small to contain an end of central directory",
        ));
    }

    for pos in (0..=tail.len() - EOCD_MIN_SIZE).rev() {
        let signature = read_u32_le(tail, pos, "EOCD signature")?;
        if signature != END_OF_CENTRAL_DIRECTORY_SIGNATURE {
            continue;
        }

        let comment_len = read_u16_le(tail, pos + 20, "EOCD comment length")? as usize;
        if pos + EOCD_MIN_SIZE + comment_len != tail.len() {
            continue;
        }

        let disk_number = read_u16_le(tail, pos + 4, "EOCD disk number")?;
        let central_directory_disk = read_u16_le(tail, pos + 6, "EOCD central directory disk")?;
        if disk_number != 0 || central_directory_disk != 0 {
            return Err(zip_error("multi-disk ZIP archives are not supported"));
        }

        return Ok(EndOfCentralDirectory {
            offset: tail_start + pos as u64,
            entries: read_u16_le(tail, pos + 10, "EOCD total entries")? as u64,
            central_directory_size: read_u32_le(tail, pos + 12, "EOCD central directory size")?
                as u64,
            central_directory_offset: read_u32_le(tail, pos + 16, "EOCD central directory offset")?
                as u64,
        });
    }

    Err(zip_error("could not find end of central directory"))
}

fn find_info_plist_entry(central_directory: &[u8]) -> Result<CentralDirectoryEntry, ClientError> {
    let mut pos = 0;
    while pos < central_directory.len() {
        let entry = parse_central_directory_entry(central_directory, pos)?;
        if is_target_info_plist(&entry.0) {
            return Ok(entry.1);
        }
        pos = entry.2;
    }

    Err(zip_error("no Info.plist found"))
}

fn parse_central_directory_entry(
    central_directory: &[u8],
    pos: usize,
) -> Result<(String, CentralDirectoryEntry, usize), ClientError> {
    let signature = read_u32_le(central_directory, pos, "central directory signature")?;
    if signature != CENTRAL_DIRECTORY_HEADER_SIGNATURE {
        return Err(zip_error("invalid central directory header"));
    }

    let flags = read_u16_le(central_directory, pos + 8, "central directory flags")?;
    let compression_method = read_u16_le(
        central_directory,
        pos + 10,
        "central directory compression method",
    )?;
    let last_mod_time = read_u16_le(
        central_directory,
        pos + 12,
        "central directory modified time",
    )?;
    let last_mod_date = read_u16_le(
        central_directory,
        pos + 14,
        "central directory modified date",
    )?;
    let file_name_len =
        read_u16_le(central_directory, pos + 28, "central directory name length")? as usize;
    let extra_field_len = read_u16_le(
        central_directory,
        pos + 30,
        "central directory extra field length",
    )? as usize;
    let comment_len = read_u16_le(
        central_directory,
        pos + 32,
        "central directory comment length",
    )? as usize;

    let file_name_start = pos + 46;
    let extra_field_start = file_name_start + file_name_len;
    let comment_start = extra_field_start + extra_field_len;
    let next = comment_start + comment_len;

    let file_name_raw = checked_slice(
        central_directory,
        file_name_start,
        file_name_len,
        "central directory file name",
    )?;
    let extra_field = checked_slice(
        central_directory,
        extra_field_start,
        extra_field_len,
        "central directory extra field",
    )?;
    checked_slice(
        central_directory,
        comment_start,
        comment_len,
        "central directory comment",
    )?;

    let file_name = String::from_utf8_lossy(file_name_raw).into_owned();
    let mut entry = CentralDirectoryEntry {
        compression_method,
        flags,
        compressed_size: read_u32_le(
            central_directory,
            pos + 20,
            "central directory compressed size",
        )? as u64,
        uncompressed_size: read_u32_le(
            central_directory,
            pos + 24,
            "central directory uncompressed size",
        )? as u64,
        local_header_offset: read_u32_le(
            central_directory,
            pos + 42,
            "central directory local header offset",
        )? as u64,
        modified: zip::DateTime::try_from_msdos(last_mod_date, last_mod_time).ok(),
    };

    apply_zip64_extra_field(extra_field, &mut entry)?;

    Ok((file_name, entry, next))
}

fn apply_zip64_extra_field(
    extra_field: &[u8],
    entry: &mut CentralDirectoryEntry,
) -> Result<(), ClientError> {
    let needs_uncompressed_size = entry.uncompressed_size == ZIP32_MAX;
    let needs_compressed_size = entry.compressed_size == ZIP32_MAX;
    let needs_local_header_offset = entry.local_header_offset == ZIP32_MAX;

    let mut pos = 0;
    while pos + 4 <= extra_field.len() {
        let header_id = read_u16_le(extra_field, pos, "extra field header id")?;
        let data_size = read_u16_le(extra_field, pos + 2, "extra field data size")? as usize;
        pos += 4;

        let data = checked_slice(extra_field, pos, data_size, "extra field data")?;
        if header_id == ZIP64_EXTRA_FIELD_ID {
            let mut data_pos = 0;
            if needs_uncompressed_size {
                entry.uncompressed_size = read_u64_le(data, data_pos, "ZIP64 uncompressed size")?;
                data_pos += 8;
            }
            if needs_compressed_size {
                entry.compressed_size = read_u64_le(data, data_pos, "ZIP64 compressed size")?;
                data_pos += 8;
            }
            if needs_local_header_offset {
                entry.local_header_offset =
                    read_u64_le(data, data_pos, "ZIP64 local header offset")?;
            }
            break;
        }

        pos += data_size;
    }

    if entry.uncompressed_size == ZIP32_MAX
        || entry.compressed_size == ZIP32_MAX
        || entry.local_header_offset == ZIP32_MAX
    {
        return Err(zip_error("missing ZIP64 extra field values"));
    }

    Ok(())
}

fn local_file_data_offset(
    local_header: &[u8],
    local_header_offset: u64,
) -> Result<u64, ClientError> {
    let signature = read_u32_le(local_header, 0, "local file header signature")?;
    if signature != LOCAL_FILE_HEADER_SIGNATURE {
        return Err(zip_error("invalid local file header"));
    }

    let file_name_len = read_u16_le(local_header, 26, "local file name length")? as u64;
    let extra_field_len = read_u16_le(local_header, 28, "local file extra field length")? as u64;
    local_header_offset
        .checked_add(30)
        .and_then(|offset| offset.checked_add(file_name_len))
        .and_then(|offset| offset.checked_add(extra_field_len))
        .ok_or_else(|| zip_error("local file data offset overflow"))
}

fn decompress_entry(
    entry: &CentralDirectoryEntry,
    compressed: &[u8],
) -> Result<Vec<u8>, ClientError> {
    if entry.flags & 1 == 1 {
        return Err(zip_error("encrypted Info.plist entries are not supported"));
    }

    let mut out = Vec::with_capacity(as_usize(
        entry.uncompressed_size.min(1024 * 1024),
        "uncompressed size",
    )?);
    match entry.compression_method {
        0 => out.extend_from_slice(compressed),
        8 => {
            let mut decoder = DeflateDecoder::new(compressed);
            decoder
                .read_to_end(&mut out)
                .map_err(|e| zip_error(format!("deflate: {e}")))?;
        }
        method => {
            return Err(zip_error(format!(
                "unsupported Info.plist compression method {method}"
            )));
        }
    }

    if out.len() as u64 != entry.uncompressed_size {
        return Err(zip_error(format!(
            "Info.plist size mismatch: expected {}, got {}",
            entry.uncompressed_size,
            out.len()
        )));
    }

    Ok(out)
}

fn is_target_info_plist(name: &str) -> bool {
    name.starts_with("Payload/")
        && name.ends_with(".app/Info.plist")
        && !name.contains("Watch/")
        && name.matches('/').count() == 2
}

fn read_u16_le(buf: &[u8], offset: usize, field: &str) -> Result<u16, ClientError> {
    Ok(u16::from_le_bytes(read_array(buf, offset, field)?))
}

fn read_u32_le(buf: &[u8], offset: usize, field: &str) -> Result<u32, ClientError> {
    Ok(u32::from_le_bytes(read_array(buf, offset, field)?))
}

fn read_u64_le(buf: &[u8], offset: usize, field: &str) -> Result<u64, ClientError> {
    Ok(u64::from_le_bytes(read_array(buf, offset, field)?))
}

fn read_array<const N: usize>(
    buf: &[u8],
    offset: usize,
    field: &str,
) -> Result<[u8; N], ClientError> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| zip_error(format!("{field} offset overflow")))?;
    let bytes = buf
        .get(offset..end)
        .ok_or_else(|| zip_error(format!("truncated {field}")))?;
    Ok(bytes.try_into().expect("slice length was checked"))
}

fn checked_slice<'a>(
    buf: &'a [u8],
    offset: usize,
    len: usize,
    field: &str,
) -> Result<&'a [u8], ClientError> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| zip_error(format!("{field} offset overflow")))?;
    buf.get(offset..end)
        .ok_or_else(|| zip_error(format!("truncated {field}")))
}

fn as_usize(value: u64, field: &str) -> Result<usize, ClientError> {
    usize::try_from(value).map_err(|_| zip_error(format!("{field} is too large")))
}

fn zip_error(message: impl Into<String>) -> ClientError {
    ClientError::UnexpectedResponse(format!("zip: {}", message.into()))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn reads_deflated_payload_info_plist_from_zip_metadata() {
        let info_plist =
            br#"<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleVersion</key><string>1</string></dict></plist>"#;
        let mut zip_data = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut zip_data);
            writer
                .start_file(
                    "Payload/Test.app/Info.plist",
                    SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .unwrap();
            writer.write_all(info_plist).unwrap();
            writer.finish().unwrap();
        }

        let zip_data = zip_data.into_inner();
        let eocd = find_eocd_in_tail(&zip_data, 0).unwrap();
        let central_directory = &zip_data[eocd.central_directory_offset as usize
            ..(eocd.central_directory_offset + eocd.central_directory_size) as usize];
        let entry = find_info_plist_entry(central_directory).unwrap();
        let local_header =
            &zip_data[entry.local_header_offset as usize..entry.local_header_offset as usize + 30];
        let data_offset = local_file_data_offset(local_header, entry.local_header_offset).unwrap();
        let compressed =
            &zip_data[data_offset as usize..(data_offset + entry.compressed_size) as usize];

        assert_eq!(decompress_entry(&entry, compressed).unwrap(), info_plist);
    }

    #[test]
    fn ignores_nested_watch_info_plist() {
        assert!(!is_target_info_plist(
            "Payload/Test.app/Watch/Test Watch.app/Info.plist"
        ));
        assert!(is_target_info_plist("Payload/Test.app/Info.plist"));
    }
}
