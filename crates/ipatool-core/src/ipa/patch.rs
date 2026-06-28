use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use crate::api::download::{DownloadItem, Sinf};
use crate::error::IpaError;

pub fn patch_ipa(
    src: &Path,
    dest: &Path,
    item: &DownloadItem,
    email: &str,
) -> Result<(), IpaError> {
    let src_file = std::fs::File::open(src)?;
    let mut src_zip = zip::ZipArchive::new(src_file)?;

    let dest_file = std::fs::File::create(dest)?;
    let mut dest_zip = zip::ZipWriter::new(dest_file);

    for i in 0..src_zip.len() {
        let mut entry = src_zip.by_index(i)?;
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(entry.compression());
        if entry.is_dir() {
            dest_zip.add_directory(entry.name().to_string(), opts)?;
        } else {
            dest_zip.start_file(entry.name().to_string(), opts)?;
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            dest_zip.write_all(&buf)?;
        }
    }

    write_itunes_metadata(&mut dest_zip, &item.metadata, email)?;
    write_sinfs(&mut dest_zip, &mut src_zip, &item.sinfs)?;

    dest_zip.finish()?;
    Ok(())
}

fn write_itunes_metadata(
    zip: &mut zip::ZipWriter<std::fs::File>,
    metadata: &HashMap<String, plist::Value>,
    email: &str,
) -> Result<(), IpaError> {
    let mut meta_dict = plist::Dictionary::new();
    for (k, v) in metadata {
        meta_dict.insert(k.clone(), v.clone());
    }
    meta_dict.insert(
        "apple-id".into(),
        plist::Value::String(email.into()),
    );
    meta_dict.insert(
        "userName".into(),
        plist::Value::String(email.into()),
    );

    let mut buf = Vec::new();
    plist::to_writer_binary(&mut buf, &meta_dict)
        .map_err(|e| IpaError::Other(format!("plist serialize: {e}")))?;

    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    zip.start_file("iTunesMetadata.plist", opts)?;
    zip.write_all(&buf)?;

    Ok(())
}

fn write_sinfs(
    dest_zip: &mut zip::ZipWriter<std::fs::File>,
    src_zip: &mut zip::ZipArchive<std::fs::File>,
    sinfs: &[Sinf],
) -> Result<(), IpaError> {
    let app_dir = find_app_dir(src_zip)?;

    let manifest_path = format!("{app_dir}SC_Info/Manifest.plist");
    if let Ok(sinf_paths) = read_manifest_sinf_paths(src_zip, &manifest_path) {
        for (i, sinf_path) in sinf_paths.iter().enumerate() {
            if let Some(sinf) = sinfs.get(i) {
                let full_path = format!("{app_dir}{sinf_path}");
                let opts = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);
                dest_zip.start_file(full_path, opts)?;
                dest_zip.write_all(&sinf.sinf)?;
            }
        }
        return Ok(());
    }

    let info_plist_path = format!("{app_dir}Info.plist");
    let executable = read_bundle_executable(src_zip, &info_plist_path)?;

    if let Some(sinf) = sinfs.first() {
        let sinf_path = format!("{app_dir}SC_Info/{executable}.sinf");
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        dest_zip.start_file(sinf_path, opts)?;
        dest_zip.write_all(&sinf.sinf)?;
    }

    Ok(())
}

fn find_app_dir(zip: &mut zip::ZipArchive<std::fs::File>) -> Result<String, IpaError> {
    for i in 0..zip.len() {
        let name = zip.by_index(i)?.name().to_string();
        if name.starts_with("Payload/") && name.ends_with(".app/") {
            let parts: Vec<&str> = name.splitn(3, '/').collect();
            if parts.len() >= 2 && !parts[1].is_empty() {
                return Ok(format!("Payload/{}/", parts[1]));
            }
        }
    }
    Err(IpaError::Other("no .app directory found in IPA".into()))
}

fn read_manifest_sinf_paths(
    zip: &mut zip::ZipArchive<std::fs::File>,
    path: &str,
) -> Result<Vec<String>, IpaError> {
    let mut entry = zip.by_name(path)?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf)?;
    drop(entry);

    let dict: plist::Dictionary =
        plist::from_bytes(&buf).map_err(|e| IpaError::Other(format!("manifest plist: {e}")))?;

    let paths = dict
        .get("SinfPaths")
        .and_then(|v| v.as_array())
        .ok_or_else(|| IpaError::Other("missing SinfPaths".into()))?;

    paths
        .iter()
        .map(|v| {
            v.as_string()
                .map(String::from)
                .ok_or_else(|| IpaError::Other("SinfPaths entry not a string".into()))
        })
        .collect()
}

fn read_bundle_executable(
    zip: &mut zip::ZipArchive<std::fs::File>,
    info_plist_path: &str,
) -> Result<String, IpaError> {
    let mut entry = zip.by_name(info_plist_path)?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf)?;
    drop(entry);

    let dict: plist::Dictionary =
        plist::from_bytes(&buf).map_err(|e| IpaError::Other(format!("Info.plist: {e}")))?;

    dict.get("CFBundleExecutable")
        .and_then(|v| v.as_string())
        .map(String::from)
        .ok_or_else(|| IpaError::Other("missing CFBundleExecutable".into()))
}
