//! Acquisition of the Apple binaries the SAP state machine is implemented by.
//!
//! The signing code is Apple's, not ours, and is not redistributed: it is
//! extracted at runtime from a public OS X 10.9 update package and verified
//! against known digests before use.
//!
//! Downloading all 1.27 GB would be wasteful, so the package is read by range
//! request. The `Payload` entry is stored in the archive uncompressed because it
//! is itself one long bzip2 stream, which makes it seekable at block
//! boundaries: starting at a known block offset and prepending a bzip2 stream
//! header resumes decompression partway through, and only ~33 MB is
//! transferred.

pub mod cpio;
pub mod xar;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bzip2::{Decompress, Status};
use futures_util::StreamExt;

use crate::client::AppleClient;
use crate::error::ClientError;
use crate::sap::hex_digest;

const UPDATE_URL: &str = "https://swcdn.apple.com/content/downloads/27/34/041-98128-A_SYPWICN3KH/5dqkl4rqgbsr18yzy61yeie9g3cmjc5hiv/OSXUpd10.9.pkg";

const PAYLOAD_ENTRY: &str = "Payload";

/// Offset of a bzip2 block boundary within the payload stream.
const PAYLOAD_BZ_OFFSET: u64 = 0x352F_40D5;

/// Distance from that block boundary to the first CPIO header.
const PAYLOAD_CPIO_SKIP: usize = 0x3A4;

/// Guards against a redirect to an error page being decompressed forever.
const MAX_TRANSFER: u64 = 256 << 20;

const FRAMEWORKS: &str = "./System/Library/PrivateFrameworks/";

struct FileSpec {
    name: &'static str,
    path: &'static str,
    size: usize,
    digest: &'static str,
}

const REQUIRED: &[FileSpec] = &[
    FileSpec {
        name: "CommerceKit",
        path: "CommerceKit.framework/Versions/A/CommerceKit",
        size: 3_271_840,
        digest: "b84ff12c21987856c0a17b78f1ad82b73195a6dec5f3b208a17d245555a2c8a2",
    },
    FileSpec {
        name: "CommerceCore",
        path: "CommerceKit.framework/Versions/A/Frameworks/CommerceCore.framework/Versions/A/CommerceCore",
        size: 207_744,
        digest: "c5401e57402230f3c876409d295319ddf1e61287bc882683c5d61277be7bc1f2",
    },
    FileSpec {
        name: "CoreFP",
        path: "CoreFP.framework/Versions/A/CoreFP",
        size: 29_014_912,
        digest: "f19141336be4198d0f8991bb00017c915efc7aeaece36c345f7faa1237ea6074",
    },
    FileSpec {
        name: "CoreFP.icxs",
        path: "CoreFP.framework/Versions/A/CoreFP.icxs",
        size: 5_288_352,
        digest: "473e78af86979f5bd4f6269561caf770b3d16c098d918846eeac8cdd2fe6566a",
    },
];

#[derive(Debug, Clone)]
pub struct Bundle {
    pub commerce_kit: Vec<u8>,
    pub commerce_core: Vec<u8>,
    pub core_fp: Vec<u8>,
    pub core_fp_icxs: Vec<u8>,
}

/// Returns the bundle from `cache_dir`, downloading and caching it on a miss.
pub async fn load(client: &AppleClient, cache_dir: &Path) -> Result<Bundle, ClientError> {
    match read_cache(cache_dir) {
        Ok(bundle) => {
            tracing::debug!("using cached Apple SAP assets");

            return Ok(bundle);
        }
        Err(e) => tracing::debug!("Apple SAP asset cache unusable: {e}"),
    }

    let bundle = download(client).await?;

    if let Err(e) = write_cache(cache_dir, &bundle) {
        // A cache miss costs a re-download, not correctness.
        tracing::warn!("failed to cache Apple SAP assets: {e}");
    }

    Ok(bundle)
}

pub async fn download(client: &AppleClient) -> Result<Bundle, ClientError> {
    let header = xar::parse_header(&range(client, 0, xar::HEADER_LEN as u64 - 1).await?)?;

    let toc = xar::inflate_toc(
        &range(client, header.size, header.size + header.toc_compressed - 1).await?,
    )?;

    let payload = xar::find_entry(&toc, PAYLOAD_ENTRY)?;
    let start = header.heap() + payload.offset + PAYLOAD_BZ_OFFSET;

    tracing::info!("downloading Apple SAP assets");

    let response = client
        .http()
        .get(UPDATE_URL)
        .header(reqwest::header::RANGE, format!("bytes={start}-"))
        .send()
        .await?;

    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(ClientError::Sap(format!(
            "Apple software update server returned HTTP {} for a range request",
            response.status()
        )));
    }

    let wanted: HashMap<String, &FileSpec> = REQUIRED
        .iter()
        .map(|spec| (format!("{FRAMEWORKS}{}", spec.path), spec))
        .collect();

    let mut decoder = Decompress::new(false);
    let mut pending = Vec::new();
    // Resume decompression mid-stream by supplying the header Apple's payload
    // has only at its very beginning.
    feed(&mut decoder, b"BZh9", &mut pending)?;

    let mut stream = response.bytes_stream();
    let mut found: HashMap<&str, Vec<u8>> = HashMap::new();
    let mut skipped = false;
    let mut transferred = 0u64;

    while found.len() < REQUIRED.len() {
        let Some(chunk) = stream.next().await else {
            break;
        };

        let chunk = chunk?;
        transferred += chunk.len() as u64;

        if transferred > MAX_TRANSFER {
            return Err(ClientError::Sap(format!(
                "Apple software update exceeded {MAX_TRANSFER} bytes without yielding the SAP assets"
            )));
        }

        if feed(&mut decoder, &chunk, &mut pending)? {
            break;
        }

        if !skipped {
            if pending.len() < PAYLOAD_CPIO_SKIP {
                continue;
            }

            pending.drain(..PAYLOAD_CPIO_SKIP);
            skipped = true;
        }

        if extract(&mut pending, &wanted, &mut found)? {
            break;
        }
    }

    let bundle = assemble(found)?;
    validate(&bundle)?;

    tracing::info!(
        "extracted Apple SAP assets ({} MB transferred)",
        transferred / (1 << 20)
    );

    Ok(bundle)
}

/// Drains complete CPIO entries from `pending`. Returns true at the trailer.
fn extract(
    pending: &mut Vec<u8>,
    wanted: &HashMap<String, &FileSpec>,
    found: &mut HashMap<&'static str, Vec<u8>>,
) -> Result<bool, ClientError> {
    loop {
        match cpio::next_entry(pending, |name| wanted.contains_key(name))? {
            cpio::Step::NeedMore(_) => return Ok(false),
            cpio::Step::End => return Ok(true),
            cpio::Step::Entry(entry) => {
                if let Some(spec) = wanted.get(entry.name.as_str()) {
                    found.insert(spec.name, entry.body);

                    if found.len() == wanted.len() {
                        return Ok(true);
                    }
                }
            }
        }
    }
}

/// Pushes `input` through the decoder, growing `output` as needed. Returns true
/// once the bzip2 stream ends.
fn feed(
    decoder: &mut Decompress,
    mut input: &[u8],
    output: &mut Vec<u8>,
) -> Result<bool, ClientError> {
    loop {
        let consumed_before = decoder.total_in();
        let produced_before = decoder.total_out();

        output.reserve(64 << 10);

        let status = decoder
            .decompress_vec(input, output)
            .map_err(|e| ClientError::Sap(format!("decompress Apple payload: {e}")))?;

        let consumed = (decoder.total_in() - consumed_before) as usize;
        let produced = decoder.total_out() - produced_before;
        input = &input[consumed..];

        if status == Status::StreamEnd {
            return Ok(true);
        }

        if input.is_empty() {
            return Ok(false);
        }

        // No forward progress with input still buffered means the decoder is
        // wedged; bail rather than spin.
        if consumed == 0 && produced == 0 {
            return Err(ClientError::Sap(
                "Apple payload decompression stalled".into(),
            ));
        }
    }
}

async fn range(client: &AppleClient, from: u64, to: u64) -> Result<Vec<u8>, ClientError> {
    let response = client
        .http()
        .get(UPDATE_URL)
        .header(reqwest::header::RANGE, format!("bytes={from}-{to}"))
        .send()
        .await?;

    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(ClientError::Sap(format!(
            "Apple software update server returned HTTP {} for a range request",
            response.status()
        )));
    }

    Ok(response.bytes().await?.to_vec())
}

fn assemble(mut found: HashMap<&'static str, Vec<u8>>) -> Result<Bundle, ClientError> {
    let mut take = |name: &str| {
        found
            .remove(name)
            .ok_or_else(|| ClientError::Sap(format!("Apple update payload has no {name}")))
    };

    Ok(Bundle {
        commerce_kit: take("CommerceKit")?,
        commerce_core: take("CommerceCore")?,
        core_fp: take("CoreFP")?,
        core_fp_icxs: take("CoreFP.icxs")?,
    })
}

fn files(bundle: &Bundle) -> Vec<(&'static str, &[u8])> {
    vec![
        ("CommerceKit", bundle.commerce_kit.as_slice()),
        ("CommerceCore", bundle.commerce_core.as_slice()),
        ("CoreFP", bundle.core_fp.as_slice()),
        ("CoreFP.icxs", bundle.core_fp_icxs.as_slice()),
    ]
}

/// These bytes are executed, so they are checked against known digests on every
/// load — after a download and again after being read back from the cache.
pub fn validate(bundle: &Bundle) -> Result<(), ClientError> {
    let contents: HashMap<&str, &[u8]> = files(bundle).into_iter().collect();

    for spec in REQUIRED {
        let data = contents
            .get(spec.name)
            .ok_or_else(|| ClientError::Sap(format!("Apple SAP asset {} is missing", spec.name)))?;

        if data.len() != spec.size {
            return Err(ClientError::Sap(format!(
                "Apple SAP asset {} has size {}, expected {}",
                spec.name,
                data.len(),
                spec.size
            )));
        }

        if hex_digest(data) != spec.digest {
            return Err(ClientError::Sap(format!(
                "Apple SAP asset {} failed integrity verification",
                spec.name
            )));
        }
    }

    Ok(())
}

fn cache_path(directory: &Path, name: &str) -> PathBuf {
    directory.join(name)
}

fn read_cache(directory: &Path) -> Result<Bundle, ClientError> {
    let mut found = HashMap::new();

    for spec in REQUIRED {
        let data = std::fs::read(cache_path(directory, spec.name)).map_err(|e| {
            ClientError::Sap(format!("read cached Apple SAP asset {}: {e}", spec.name))
        })?;

        found.insert(spec.name, data);
    }

    let bundle = assemble(found)?;
    validate(&bundle)?;

    Ok(bundle)
}

fn write_cache(directory: &Path, bundle: &Bundle) -> Result<(), ClientError> {
    std::fs::create_dir_all(directory)
        .map_err(|e| ClientError::Sap(format!("create SAP asset cache: {e}")))?;

    for (name, data) in files(bundle) {
        let path = cache_path(directory, name);
        let temporary = path.with_extension("partial");

        std::fs::write(&temporary, data)
            .map_err(|e| ClientError::Sap(format!("write cached SAP asset {name}: {e}")))?;

        std::fs::rename(&temporary, &path)
            .map_err(|e| ClientError::Sap(format!("install cached SAP asset {name}: {e}")))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle_of(sizes: [usize; 4]) -> Bundle {
        Bundle {
            commerce_kit: vec![0; sizes[0]],
            commerce_core: vec![0; sizes[1]],
            core_fp: vec![0; sizes[2]],
            core_fp_icxs: vec![0; sizes[3]],
        }
    }

    #[test]
    fn rejects_wrong_sizes_before_hashing() {
        assert!(validate(&bundle_of([1, 1, 1, 1])).is_err());
    }

    /// Right length, wrong contents — only the digest can catch this.
    #[test]
    fn rejects_correctly_sized_but_wrong_contents() {
        let bundle = bundle_of([3_271_840, 207_744, 29_014_912, 5_288_352]);

        let message = validate(&bundle).unwrap_err().to_string();
        assert!(
            message.contains("failed integrity verification"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn assemble_reports_the_missing_file() {
        let found = HashMap::from([("CommerceKit", vec![0u8; 4])]);

        assert!(assemble(found).unwrap_err().to_string().contains("has no"));
    }

    /// Hits Apple's software-update CDN and transfers ~33 MB, so it is opt-in:
    /// `cargo test -p ipatool-core -- --ignored live_download`.
    #[tokio::test]
    #[ignore = "requires network access to swcdn.apple.com"]
    async fn live_download_yields_verified_assets() {
        let client = crate::client::AppleClient::for_tests();

        let bundle = download(&client).await.expect("download SAP assets");

        // download() validates internally; assert the shape too so a future
        // refactor that drops that check still fails here.
        validate(&bundle).unwrap();
        assert_eq!(bundle.core_fp.len(), 29_014_912);
    }
}
