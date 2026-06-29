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
        let entry = src_zip.by_index(i)?;
        if entry.is_dir() {
            let opts =
                zip::write::SimpleFileOptions::default().compression_method(entry.compression());
            dest_zip.add_directory(entry.name().to_string(), opts)?;
        } else {
            dest_zip.raw_copy_file(entry)?;
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
    meta_dict.insert("apple-id".into(), plist::Value::String(email.into()));
    meta_dict.insert("userName".into(), plist::Value::String(email.into()));

    let mut buf = Vec::new();
    plist::to_writer_binary(&mut buf, &meta_dict)
        .map_err(|e| IpaError::Other(format!("plist serialize: {e}")))?;

    let opts =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
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
    match read_manifest_sinf_paths(src_zip, &manifest_path) {
        Ok(sinf_paths) => {
            if sinf_paths.len() != sinfs.len() {
                return Err(IpaError::Other(format!(
                    "sinf count mismatch: manifest has {}, response has {}",
                    sinf_paths.len(),
                    sinfs.len()
                )));
            }

            for (sinf, sinf_path) in sinfs.iter().zip(sinf_paths.iter()) {
                let full_path = format!("{app_dir}{sinf_path}");
                write_sinf_file(dest_zip, &full_path, sinf)?;
            }
            return Ok(());
        }
        Err(IpaError::Zip(zip::result::ZipError::FileNotFound)) => {}
        Err(err) => return Err(err),
    }

    let info_plist_path = format!("{app_dir}Info.plist");
    let executable = read_bundle_executable(src_zip, &info_plist_path)?;

    let sinf = sinfs
        .first()
        .ok_or_else(|| IpaError::Other("missing sinf data".into()))?;
    let sinf_path = format!("{app_dir}SC_Info/{executable}.sinf");
    write_sinf_file(dest_zip, &sinf_path, sinf)?;

    Ok(())
}

fn find_app_dir(zip: &mut zip::ZipArchive<std::fs::File>) -> Result<String, IpaError> {
    for i in 0..zip.len() {
        let name = zip.by_index(i)?.name().to_string();
        let Some(rest) = name.strip_prefix("Payload/") else {
            continue;
        };
        let Some((app_name, file_name)) = rest.split_once('/') else {
            continue;
        };
        if app_name.ends_with(".app") && file_name == "Info.plist" {
            return Ok(format!("Payload/{app_name}/"));
        }
    }
    Err(IpaError::Other("no .app directory found in IPA".into()))
}

fn write_sinf_file(
    zip: &mut zip::ZipWriter<std::fs::File>,
    path: &str,
    sinf: &Sinf,
) -> Result<(), IpaError> {
    let opts =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file(path, opts)?;
    zip.write_all(&sinf.sinf)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn item_with_sinfs(sinfs: Vec<Sinf>) -> DownloadItem {
        DownloadItem {
            url: "https://example.invalid/app.ipa".into(),
            sinfs,
            metadata: HashMap::new(),
        }
    }

    fn sinf(bytes: &[u8]) -> Sinf {
        Sinf {
            id: 1,
            sinf: bytes.to_vec(),
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ipatool-rs-{name}-{}-{nanos}.ipa",
            std::process::id()
        ))
    }

    fn write_info_plist_with_executable(
        writer: &mut zip::ZipWriter<File>,
        path: &str,
        executable: &str,
    ) {
        let mut info = plist::Dictionary::new();
        info.insert(
            "CFBundleExecutable".into(),
            plist::Value::String(executable.into()),
        );
        let mut buf = Vec::new();
        plist::to_writer_binary(&mut buf, &info).unwrap();

        writer
            .start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&buf).unwrap();
    }

    fn write_info_plist(writer: &mut zip::ZipWriter<File>, path: &str) {
        write_info_plist_with_executable(writer, path, "DemoExec");
    }

    fn write_manifest(writer: &mut zip::ZipWriter<File>, paths: &[&str]) {
        let mut manifest = plist::Dictionary::new();
        manifest.insert(
            "SinfPaths".into(),
            plist::Value::Array(
                paths
                    .iter()
                    .map(|path| plist::Value::String((*path).into()))
                    .collect(),
            ),
        );
        let mut buf = Vec::new();
        plist::to_writer_binary(&mut buf, &manifest).unwrap();

        writer
            .start_file(
                "Payload/Demo.app/SC_Info/Manifest.plist",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(&buf).unwrap();
    }

    fn write_malformed_manifest(writer: &mut zip::ZipWriter<File>) {
        writer
            .start_file(
                "Payload/Demo.app/SC_Info/Manifest.plist",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"not a plist").unwrap();
    }

    fn write_base_ipa(path: &Path, manifest_paths: Option<&[&str]>) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);

        writer
            .start_file(
                "Payload/Demo.app/DemoExec",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .unix_permissions(0o755),
            )
            .unwrap();
        writer.write_all(b"demo executable").unwrap();

        write_info_plist(&mut writer, "Payload/Demo.app/Info.plist");

        if let Some(paths) = manifest_paths {
            write_manifest(&mut writer, paths);
        }

        writer.finish().unwrap();
    }

    fn write_ipa_with_explicit_directories(path: &Path) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);

        let dir_opts = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        writer.add_directory("Payload/", dir_opts).unwrap();
        writer.add_directory("Payload/Demo.app/", dir_opts).unwrap();

        writer
            .start_file(
                "Payload/Demo.app/DemoExec",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .unix_permissions(0o755),
            )
            .unwrap();
        writer.write_all(b"demo executable").unwrap();

        write_info_plist(&mut writer, "Payload/Demo.app/Info.plist");

        writer.finish().unwrap();
    }

    fn write_ipa_with_malformed_manifest(path: &Path) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);

        writer
            .start_file(
                "Payload/Demo.app/DemoExec",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"demo executable").unwrap();

        write_info_plist(&mut writer, "Payload/Demo.app/Info.plist");
        write_malformed_manifest(&mut writer);

        writer.finish().unwrap();
    }

    fn write_ipa_with_nested_app_before_top_level(path: &Path) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);

        write_info_plist_with_executable(
            &mut writer,
            "Payload/Demo.app/PlugIns/Clip.app/Info.plist",
            "ClipExec",
        );

        writer
            .start_file(
                "Payload/Demo.app/DemoExec",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"demo executable").unwrap();

        write_info_plist(&mut writer, "Payload/Demo.app/Info.plist");

        writer.finish().unwrap();
    }

    #[test]
    fn patches_ipa_without_explicit_app_directory_entry() {
        let src = temp_path("src-no-dir");
        let dest = temp_path("dest-no-dir");
        write_base_ipa(&src, None);

        patch_ipa(
            &src,
            &dest,
            &item_with_sinfs(vec![sinf(b"fallback-sinf")]),
            "user@example.com",
        )
        .unwrap();

        let file = File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        archive
            .by_name("Payload/Demo.app/SC_Info/DemoExec.sinf")
            .unwrap();
        archive.by_name("iTunesMetadata.plist").unwrap();

        std::fs::remove_file(src).ok();
        std::fs::remove_file(dest).ok();
    }

    #[test]
    fn rejects_malformed_manifest_instead_of_fallback() {
        let src = temp_path("src-bad-manifest");
        let dest = temp_path("dest-bad-manifest");
        write_ipa_with_malformed_manifest(&src);

        let err = patch_ipa(
            &src,
            &dest,
            &item_with_sinfs(vec![sinf(b"fallback-sinf")]),
            "user@example.com",
        )
        .unwrap_err();

        assert!(err.to_string().contains("manifest plist"));

        std::fs::remove_file(src).ok();
        std::fs::remove_file(dest).ok();
    }

    #[test]
    fn rejects_manifest_sinf_count_mismatch() {
        let src = temp_path("src-mismatch");
        let dest = temp_path("dest-mismatch");
        write_base_ipa(&src, Some(&["SC_Info/DemoExec.sinf", "SC_Info/Extra.sinf"]));

        let err = patch_ipa(
            &src,
            &dest,
            &item_with_sinfs(vec![sinf(b"only-one")]),
            "user@example.com",
        )
        .unwrap_err();

        assert!(err.to_string().contains("sinf count mismatch"));

        std::fs::remove_file(src).ok();
        std::fs::remove_file(dest).ok();
    }

    #[test]
    fn chooses_top_level_app_when_nested_app_appears_first() {
        let src = temp_path("src-nested-app-first");
        let dest = temp_path("dest-nested-app-first");
        write_ipa_with_nested_app_before_top_level(&src);

        patch_ipa(
            &src,
            &dest,
            &item_with_sinfs(vec![sinf(b"top-level-sinf")]),
            "user@example.com",
        )
        .unwrap();

        let file = File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut top_level_sinf = archive
            .by_name("Payload/Demo.app/SC_Info/DemoExec.sinf")
            .unwrap();
        let mut data = Vec::new();
        top_level_sinf.read_to_end(&mut data).unwrap();
        drop(top_level_sinf);

        assert_eq!(data, b"top-level-sinf");
        assert!(
            archive
                .by_name("Payload/Demo.app/PlugIns/Clip.app/SC_Info/ClipExec.sinf")
                .is_err()
        );

        std::fs::remove_file(src).ok();
        std::fs::remove_file(dest).ok();
    }

    #[test]
    fn preserves_explicit_directory_entry_metadata() {
        let src = temp_path("src-explicit-dirs");
        let dest = temp_path("dest-explicit-dirs");
        write_ipa_with_explicit_directories(&src);

        patch_ipa(
            &src,
            &dest,
            &item_with_sinfs(vec![sinf(b"fallback-sinf")]),
            "user@example.com",
        )
        .unwrap();

        let file = File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let app_dir = archive.by_name("Payload/Demo.app/").unwrap();

        assert_eq!(app_dir.unix_mode().unwrap() & 0o170000, 0o040000);
        assert_eq!(app_dir.unix_mode().unwrap() & 0o777, 0o755);

        std::fs::remove_file(src).ok();
        std::fs::remove_file(dest).ok();
    }

    #[test]
    fn raw_copies_existing_entries_and_preserves_metadata() {
        let src = temp_path("src-raw");
        let dest = temp_path("dest-raw");
        write_base_ipa(&src, None);

        patch_ipa(
            &src,
            &dest,
            &item_with_sinfs(vec![sinf(b"fallback-sinf")]),
            "user@example.com",
        )
        .unwrap();

        let file = File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut executable = archive.by_name("Payload/Demo.app/DemoExec").unwrap();
        let mut data = Vec::new();
        executable.read_to_end(&mut data).unwrap();

        assert_eq!(data, b"demo executable");
        assert_eq!(executable.compression(), zip::CompressionMethod::Deflated);
        assert_eq!(executable.unix_mode().unwrap() & 0o777, 0o755);

        std::fs::remove_file(src).ok();
        std::fs::remove_file(dest).ok();
    }
}
