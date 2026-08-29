//! Acquisition and loading of the prebuilt Unicorn shared library.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use libloading::Library;

use super::artifact::{self, Artifact, VERSION};
use crate::client::AppleClient;
use crate::error::ClientError;
use crate::sap::hex_digest;

/// The wheels are ~12 MB; anything much larger is not what we pinned.
const MAX_ARCHIVE: usize = 64 << 20;

/// Returns the path of the cached library, downloading it if necessary.
pub async fn ensure(client: &AppleClient, cache_dir: &Path) -> Result<PathBuf, ClientError> {
    let artifact = artifact::current()?;
    let directory = cache_dir.join(format!("unicorn-{VERSION}"));
    let path = directory.join(artifact.filename);

    match verified_cache(&path, artifact) {
        Ok(()) => {
            tracing::debug!("using cached Unicorn library");

            return Ok(path);
        }
        Err(e) => tracing::debug!("Unicorn library cache unusable: {e}"),
    }

    let library = download(client, artifact).await?;

    std::fs::create_dir_all(&directory)
        .map_err(|e| ClientError::Sap(format!("create Unicorn cache: {e}")))?;

    let temporary = path.with_extension("partial");
    std::fs::write(&temporary, &library)
        .map_err(|e| ClientError::Sap(format!("write Unicorn library: {e}")))?;
    std::fs::rename(&temporary, &path)
        .map_err(|e| ClientError::Sap(format!("install Unicorn library: {e}")))?;

    Ok(path)
}

async fn download(client: &AppleClient, artifact: &Artifact) -> Result<Vec<u8>, ClientError> {
    tracing::info!("downloading Unicorn {VERSION}");

    let response = client.http().get(artifact.url).send().await?;

    if !response.status().is_success() {
        return Err(ClientError::Sap(format!(
            "Unicorn download returned HTTP {}",
            response.status()
        )));
    }

    let archive = response.bytes().await?;

    if archive.len() > MAX_ARCHIVE {
        return Err(ClientError::Sap(format!(
            "Unicorn archive is {} bytes, more than the {MAX_ARCHIVE} expected",
            archive.len()
        )));
    }

    // The archive is verified before it is parsed, so a substituted download
    // never reaches the zip reader.
    expect_digest(&archive, artifact.archive_digest, "Unicorn archive")?;

    let mut zip = zip::ZipArchive::new(Cursor::new(archive.as_ref()))
        .map_err(|e| ClientError::Sap(format!("open Unicorn archive: {e}")))?;

    let mut member = zip.by_name(artifact.member).map_err(|e| {
        ClientError::Sap(format!("Unicorn archive has no {}: {e}", artifact.member))
    })?;

    let mut library = Vec::with_capacity(member.size() as usize);
    member
        .read_to_end(&mut library)
        .map_err(|e| ClientError::Sap(format!("extract Unicorn library: {e}")))?;

    expect_digest(&library, artifact.library_digest, "Unicorn library")?;

    Ok(library)
}

fn verified_cache(path: &Path, artifact: &Artifact) -> Result<(), ClientError> {
    let data = std::fs::read(path)
        .map_err(|e| ClientError::Sap(format!("read cached Unicorn library: {e}")))?;

    expect_digest(&data, artifact.library_digest, "cached Unicorn library")
}

fn expect_digest(data: &[u8], expected: &str, label: &str) -> Result<(), ClientError> {
    if hex_digest(data) != expected {
        return Err(ClientError::Sap(format!(
            "{label} failed integrity verification"
        )));
    }

    Ok(())
}

/// Opens the library at `path`.
///
/// # Safety
///
/// Loading a shared library runs its initializers. The path must be one
/// produced by [`ensure`], whose contents have been verified against a pinned
/// digest.
pub unsafe fn open(path: &Path) -> Result<Library, ClientError> {
    unsafe { Library::new(path) }
        .map_err(|e| ClientError::Sap(format!("load Unicorn library {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_digest_mismatch() {
        let error = expect_digest(b"not the library", "00".repeat(32).as_str(), "test")
            .unwrap_err()
            .to_string();

        assert!(error.contains("failed integrity verification"), "{error}");
    }

    #[test]
    fn accepts_a_matching_digest() {
        let data = b"payload";

        assert!(expect_digest(data, &hex_digest(data), "test").is_ok());
    }
}
