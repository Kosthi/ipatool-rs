//! Acquisition and loading of the prebuilt Unicorn shared library.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use libloading::Library;

use super::artifact::{self, Artifact, Format, Patch, VERSION};
use super::patch;
use crate::client::AppleClient;
use crate::error::ClientError;
use crate::sap::hex_digest;

/// The largest of these archives is ~12 MB; anything much bigger is not what
/// we pinned.
const MAX_ARCHIVE: usize = 64 << 20;

/// Where a patch is needed, the digest-verified original is kept beside the
/// file that actually gets loaded, so the pinned digest stays checkable.
const ORIGINAL_SUFFIX: &str = ".original";

/// Everything that must be loaded, in order: dependencies first.
#[derive(Debug, Clone)]
pub struct Paths {
    pub dependencies: Vec<PathBuf>,
    pub library: PathBuf,
}

/// Returns the cached paths, downloading anything missing.
pub async fn ensure(client: &AppleClient, cache_dir: &Path) -> Result<Paths, ClientError> {
    let artifact = artifact::current()?;
    let directory = cache_dir.join(format!("unicorn-{VERSION}"));

    let mut dependencies = Vec::with_capacity(artifact.dependencies.len());

    for dependency in artifact.dependencies {
        dependencies.push(ensure_one(client, &directory, dependency).await?);
    }

    Ok(Paths {
        dependencies,
        library: ensure_one(client, &directory, artifact).await?,
    })
}

async fn ensure_one(
    client: &AppleClient,
    directory: &Path,
    artifact: &Artifact,
) -> Result<PathBuf, ClientError> {
    let load_path = directory.join(artifact.filename);
    let original_path = match artifact.patch {
        Some(_) => directory.join(format!("{}{ORIGINAL_SUFFIX}", artifact.filename)),
        None => load_path.clone(),
    };

    let library = match verified(&original_path, artifact.library_digest) {
        Ok(library) => {
            tracing::debug!("using cached {}", artifact.filename);

            // The patched file is derived, not pinned, so it is rebuilt rather
            // than trusted whenever it is missing.
            if artifact.patch.is_some() && load_path.exists() {
                return Ok(load_path);
            }

            library
        }
        Err(e) => {
            tracing::debug!("{} cache unusable: {e}", artifact.filename);

            let library = download(client, artifact).await?;

            std::fs::create_dir_all(directory)
                .map_err(|e| ClientError::Sap(format!("create Unicorn cache: {e}")))?;
            write(&original_path, &library)?;

            library
        }
    };

    if let Some(kind) = artifact.patch {
        let mut patched = library;

        match kind {
            Patch::WindowsArm64TcgMasks => patch::patch_windows_arm64_tcg_masks(&mut patched)?,
        }

        write(&load_path, &patched)?;
    }

    Ok(load_path)
}

async fn download(client: &AppleClient, artifact: &Artifact) -> Result<Vec<u8>, ClientError> {
    tracing::info!("downloading {} (Unicorn {VERSION})", artifact.filename);

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
    // never reaches a decompressor.
    expect_digest(&archive, artifact.archive_digest, "Unicorn archive")?;

    let library = extract(&archive, artifact)?;

    expect_digest(&library, artifact.library_digest, artifact.filename)?;

    Ok(library)
}

/// Pulls `artifact.member` out of the archive.
pub fn extract(archive: &[u8], artifact: &Artifact) -> Result<Vec<u8>, ClientError> {
    match artifact.format {
        Format::Zip => extract_zip(archive, artifact.member),
        Format::SevenZip => extract_7z(archive, artifact.member),
        Format::TarZstd => extract_tar_zstd(archive, artifact.member),
    }
    .ok_or_else(|| ClientError::Sap(format!("archive has no {}", artifact.member)))?
}

type Extracted = Option<Result<Vec<u8>, ClientError>>;

fn extract_zip(archive: &[u8], member: &str) -> Extracted {
    let mut zip = match zip::ZipArchive::new(Cursor::new(archive)) {
        Ok(zip) => zip,
        Err(e) => return Some(Err(ClientError::Sap(format!("open zip archive: {e}")))),
    };

    let mut entry = zip.by_name(member).ok()?;
    let mut out = Vec::with_capacity(entry.size() as usize);

    Some(
        entry
            .read_to_end(&mut out)
            .map(|_| out)
            .map_err(|e| ClientError::Sap(format!("extract {member}: {e}"))),
    )
}

fn extract_7z(archive: &[u8], member: &str) -> Extracted {
    let mut reader = match sevenz_rust2::ArchiveReader::new(
        Cursor::new(archive),
        sevenz_rust2::Password::empty(),
    ) {
        Ok(reader) => reader,
        Err(e) => return Some(Err(ClientError::Sap(format!("open 7z archive: {e}")))),
    };

    // Distinguishes "no such entry" from a real read failure, so a renamed
    // member in a future release is reported as such.
    if !reader
        .archive()
        .files
        .iter()
        .any(|entry| entry.name == member)
    {
        return None;
    }

    Some(
        reader
            .read_file(member)
            .map_err(|e| ClientError::Sap(format!("extract {member}: {e}"))),
    )
}

fn extract_tar_zstd(archive: &[u8], member: &str) -> Extracted {
    let decoder = match ruzstd::decoding::StreamingDecoder::new(Cursor::new(archive)) {
        Ok(decoder) => decoder,
        Err(e) => return Some(Err(ClientError::Sap(format!("open zstd stream: {e}")))),
    };

    let mut tar = tar::Archive::new(decoder);

    let entries = match tar.entries() {
        Ok(entries) => entries,
        Err(e) => return Some(Err(ClientError::Sap(format!("read tar archive: {e}")))),
    };

    for entry in entries {
        let mut entry = match entry {
            Ok(entry) => entry,
            Err(e) => return Some(Err(ClientError::Sap(format!("read tar entry: {e}")))),
        };

        let path = match entry.path() {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(e) => return Some(Err(ClientError::Sap(format!("read tar entry path: {e}")))),
        };

        if path != member {
            continue;
        }

        let mut out = Vec::new();

        return Some(
            entry
                .read_to_end(&mut out)
                .map(|_| out)
                .map_err(|e| ClientError::Sap(format!("extract {member}: {e}"))),
        );
    }

    None
}

fn write(path: &Path, data: &[u8]) -> Result<(), ClientError> {
    let temporary = path.with_extension("partial");

    std::fs::write(&temporary, data)
        .map_err(|e| ClientError::Sap(format!("write {}: {e}", path.display())))?;
    std::fs::rename(&temporary, path)
        .map_err(|e| ClientError::Sap(format!("install {}: {e}", path.display())))?;

    Ok(())
}

fn verified(path: &Path, digest: &str) -> Result<Vec<u8>, ClientError> {
    let data = std::fs::read(path)
        .map_err(|e| ClientError::Sap(format!("read {}: {e}", path.display())))?;

    expect_digest(&data, digest, "cached Unicorn library")?;

    Ok(data)
}

fn expect_digest(data: &[u8], expected: &str, label: &str) -> Result<(), ClientError> {
    if hex_digest(data) != expected {
        return Err(ClientError::Sap(format!(
            "{label} failed integrity verification"
        )));
    }

    Ok(())
}

/// Opens the library, and any dependency it needs loaded first.
///
/// # Safety
///
/// Loading a shared library runs its initializers. The paths must be ones
/// produced by [`ensure`], whose contents have been verified against pinned
/// digests.
pub unsafe fn open(paths: &Paths) -> Result<(Vec<Library>, Library), ClientError> {
    // Windows needs one more step that is not implemented: after the DLL is
    // loaded, its `longjmp` import has to be redirected, because Windows' SEH
    // longjmp cannot unwind the frames Unicorn's generated code produces and
    // will crash rather than return. Failing here is better than loading a
    // library that will fault partway through a signing call.
    #[cfg(target_os = "windows")]
    {
        let _ = paths;

        return Err(ClientError::Sap(
            "Unicorn support on Windows is incomplete: the loaded DLL still needs its longjmp \
             import redirected. See https://github.com/Kosthi/ipatool-rs/issues/15"
                .into(),
        ));
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut dependencies = Vec::with_capacity(paths.dependencies.len());

        for path in &paths.dependencies {
            dependencies.push(unsafe { open_one(path) }?);
        }

        let library = unsafe { open_one(&paths.library) }?;

        Ok((dependencies, library))
    }
}

#[cfg(not(target_os = "windows"))]
unsafe fn open_one(path: &Path) -> Result<Library, ClientError> {
    unsafe { Library::new(path) }
        .map_err(|e| ClientError::Sap(format!("load {}: {e}", path.display())))
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

    /// Downloads every platform's artifact — including the ones this machine
    /// cannot load — and checks that each archive matches its digest, yields
    /// the member it claims to, and that the extracted library matches its own
    /// digest. For Windows ARM64 the TCG patch is applied too, which is what
    /// confirms the pinned build still has the layout the patch expects.
    ///
    /// This is the only check the Windows entries get without a Windows
    /// machine: the archive handling and the patch are verified here, but
    /// loading the DLL is not.
    ///
    /// `cargo test -p ipatool-core -- --ignored live_every_platform`
    #[tokio::test]
    #[ignore = "downloads every platform's Unicorn build (~60 MB)"]
    async fn live_every_platform_artifact_is_extractable() {
        let client = crate::client::AppleClient::for_tests();

        for artifact in artifact::ALL {
            for artifact in std::iter::once(*artifact).chain(artifact.dependencies) {
                let library = download(&client, artifact)
                    .await
                    .unwrap_or_else(|e| panic!("{}: {e}", artifact.url));

                assert!(!library.is_empty(), "{}", artifact.url);

                if let Some(Patch::WindowsArm64TcgMasks) = artifact.patch {
                    let mut patched = library.clone();
                    patch::patch_windows_arm64_tcg_masks(&mut patched)
                        .unwrap_or_else(|e| panic!("patch {}: {e}", artifact.filename));

                    assert_ne!(patched, library, "the patch changed nothing");
                }
            }
        }
    }
}
