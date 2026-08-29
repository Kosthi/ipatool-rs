//! Pinned locations of prebuilt Unicorn libraries.
//!
//! Unicorn is not a build dependency: compiling its vendored QEMU needs a C
//! toolchain and breaks on newer compilers, and requiring a system libunicorn
//! would push that problem onto every user. Instead a prebuilt library is
//! fetched at runtime, exactly as the Apple assets are.
//!
//! The Python wheels on PyPI are the most convenient source on macOS and Linux
//! — a wheel is a zip archive containing a ready-built shared library. Windows
//! has no wheel carrying the DLL, so it comes from the project's own release
//! and from MSYS2. Every archive and every library it yields is pinned by
//! digest.

use crate::error::ClientError;

pub const VERSION: &str = "2.1.4";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Python wheels, and anything else that is a zip.
    Zip,
    SevenZip,
    TarZstd,
}

/// A binary patch applied to the extracted library before it is loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Patch {
    /// Unicorn's AArch64 TCG backend was built with Windows' 32-bit `long`,
    /// truncating two 64-bit masks.
    WindowsArm64TcgMasks,
}

pub struct Artifact {
    pub url: &'static str,
    pub archive_digest: &'static str,
    /// Digest of the library as extracted, before any patch is applied.
    pub library_digest: &'static str,
    /// Path of the shared library inside the archive.
    pub member: &'static str,
    /// Name the library is cached under.
    pub filename: &'static str,
    pub format: Format,
    pub patch: Option<Patch>,
    /// Libraries that must be loaded before this one.
    pub dependencies: &'static [Artifact],
}

/// Returns the artifact for the platform this binary was built for.
pub fn current() -> Result<&'static Artifact, ClientError> {
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Ok(&MACOS_X86_64);
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Ok(&MACOS_ARM64);
    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    return Ok(&LINUX_X86_64);
    #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
    return Ok(&LINUX_ARM64);
    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "musl"))]
    return Ok(&LINUX_MUSL_X86_64);
    #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "musl"))]
    return Ok(&LINUX_MUSL_ARM64);
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return Ok(&WINDOWS_X86_64);
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    return Ok(&WINDOWS_ARM64);

    #[allow(unreachable_code)]
    Err(ClientError::Sap(format!(
        "no prebuilt Unicorn {VERSION} is configured for {}/{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )))
}

const DYLIB: &str = "unicorn/lib/libunicorn.2.dylib";
const SO: &str = "unicorn/lib/libunicorn.so.2";

/// The table is kept whole rather than compiled per platform so the entries for
/// other platforms stay visible to tests and to anyone reading it.
pub static MACOS_X86_64: Artifact = Artifact {
    url: "https://files.pythonhosted.org/packages/c8/a7/92b47771e2107a201632a199cec91e8a81ee8a071ca6b7e7d600d8c61ac9/unicorn-2.1.4-cp37-abi3-macosx_10_9_x86_64.whl",
    archive_digest: "2a6f738fab5fabffa56af1e7bbf16ea1e91466c342f8dc64f125bd70f36c6b80",
    library_digest: "51c4a6f3ce22628ecd3acd1c49b921a818ffb989ca2c473134cc7eb06094f256",
    member: DYLIB,
    filename: "libunicorn.2.dylib",
    format: Format::Zip,
    patch: None,
    dependencies: &[],
};

pub static MACOS_ARM64: Artifact = Artifact {
    url: "https://files.pythonhosted.org/packages/6c/ae/4943c6f8524d729ec7d5e69df6407ea05d710fe77471d91cecf3fc64eb57/unicorn-2.1.4-cp37-abi3-macosx_11_0_arm64.whl",
    archive_digest: "d6c93e0f60328d8f4a1792af3f834137a28050fcc2305f2ec01efe8558a9844e",
    library_digest: "7207c8e3d7a63118fb0bca73e01816797fd51b1d8a39a4cbc7abfd562ee59c85",
    member: DYLIB,
    filename: "libunicorn.2.dylib",
    format: Format::Zip,
    patch: None,
    dependencies: &[],
};

pub static LINUX_X86_64: Artifact = Artifact {
    url: "https://files.pythonhosted.org/packages/e7/df/ded5e3684c2d7600b30cc8a7530277b8cb36644a1a9d34cade7ebb45604c/unicorn-2.1.4-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl",
    archive_digest: "9d6e6dea140560de4ebd8446661f7ef84a357d428c14a3ef09dacd306ec8c239",
    library_digest: "ddb196ec82b52e502c18e4a34478bf7b9f61c83c2ebaa95c74d8ded45a95da9c",
    member: SO,
    filename: "libunicorn.so.2",
    format: Format::Zip,
    patch: None,
    dependencies: &[],
};

pub static LINUX_ARM64: Artifact = Artifact {
    url: "https://files.pythonhosted.org/packages/33/9f/32d41eb942221bcf4417cdc65537fc8b3bbbd6079d6c161e621f1dd4e94a/unicorn-2.1.4-cp37-abi3-manylinux_2_17_aarch64.manylinux2014_aarch64.whl",
    archive_digest: "bd1fb0c9af5f57e356d8a96928b4fe045b2e18f308ef23b481d5f970008aa722",
    library_digest: "a0b99458a82e268aee258205a40590411c3a9f28e42abf2942ce4e87b7d9ac65",
    member: SO,
    filename: "libunicorn.so.2",
    format: Format::Zip,
    patch: None,
    dependencies: &[],
};

pub static LINUX_MUSL_X86_64: Artifact = Artifact {
    url: "https://files.pythonhosted.org/packages/ed/4b/4628ccb20eb3ad1af400de8181d1f4e5c1a3fc2affa1b3410c1b2d71af36/unicorn-2.1.4-cp37-abi3-musllinux_1_2_x86_64.whl",
    archive_digest: "d348a90ee90219d141cb115ef8ed7e3fd1af42afaee105f7580761d775b25e32",
    library_digest: "cc1a208c69b151fdd23439736b0fac9ac6e14409dae77deee900369e6daab302",
    member: SO,
    filename: "libunicorn.so.2",
    format: Format::Zip,
    patch: None,
    dependencies: &[],
};

pub static LINUX_MUSL_ARM64: Artifact = Artifact {
    url: "https://files.pythonhosted.org/packages/70/38/ba5a051c844026e59ab6e0017db8cec77dbe20ab5f1d6edae1ce9d885b06/unicorn-2.1.4-cp37-abi3-musllinux_1_2_aarch64.whl",
    archive_digest: "01d744ba01c5cc68f1d7afe3d183f1868720fd440ec4eaedc4d1d5d9bf54b84c",
    library_digest: "52179305928b32c937d2d527ad6fef9d500c6fa7cdb14bf32abf7021d67271a2",
    member: SO,
    filename: "libunicorn.so.2",
    format: Format::Zip,
    patch: None,
    dependencies: &[],
};

pub static WINDOWS_X86_64: Artifact = Artifact {
    url: "https://github.com/unicorn-engine/unicorn/releases/download/2.1.4/windows-mingw64-shared.7z",
    archive_digest: "0960f938e66fa12c448742bddd2a03aa88abeeb2b3cda7156493a2da86228d3a",
    library_digest: "d8f9a89222ffa74493a1d47090e17f8e1db8ac171a3128c6a76a4ea09de11469",
    member: "bin/libunicorn.dll",
    filename: "libunicorn.dll",
    format: Format::SevenZip,
    patch: None,
    dependencies: &[],
};

pub static WINDOWS_ARM64: Artifact = Artifact {
    url: "https://mirror.msys2.org/mingw/clangarm64/mingw-w64-clang-aarch64-unicorn-2.1.4-5-any.pkg.tar.zst",
    archive_digest: "e28aab2165d9cff048c29c58d6a40eb97928b23cf8ddeb78056a4b5b9805ac61",
    library_digest: "0ee1ebab91653645ef2b0615a8225123f7af9a49df4b6fcc5fb5d45d540ae9c2",
    member: "clangarm64/bin/libunicorn.dll",
    filename: "libunicorn.dll",
    format: Format::TarZstd,
    patch: Some(Patch::WindowsArm64TcgMasks),
    dependencies: &[Artifact {
        url: "https://mirror.msys2.org/mingw/clangarm64/mingw-w64-clang-aarch64-libwinpthread-14.0.0.r302.gd7f3c5201-1-any.pkg.tar.zst",
        archive_digest: "dd20ad17543608915a2ff9ef6f39146d5621298531e0f50706fd1e78bf1da834",
        library_digest: "b80722a2586c0d1de605724569a564f3c139d184deaa33b7df7415477d733467",
        member: "clangarm64/bin/libwinpthread-1.dll",
        filename: "libwinpthread-1.dll",
        format: Format::TarZstd,
        patch: None,
        dependencies: &[],
    }],
};

/// Every platform this crate knows how to run on, for tests that check the
/// table as a whole rather than just the running platform's entry.
pub static ALL: &[&Artifact] = &[
    &MACOS_X86_64,
    &MACOS_ARM64,
    &LINUX_X86_64,
    &LINUX_ARM64,
    &LINUX_MUSL_X86_64,
    &LINUX_MUSL_ARM64,
    &WINDOWS_X86_64,
    &WINDOWS_ARM64,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_running_platform_is_covered() {
        let artifact = current().expect("this platform should be in the table");

        assert!(artifact.url.starts_with("https://"));
    }

    #[test]
    fn every_entry_is_pinned() {
        fn check(artifact: &Artifact) {
            assert!(artifact.url.starts_with("https://"), "{}", artifact.url);
            assert_eq!(artifact.archive_digest.len(), 64, "{}", artifact.url);
            assert_eq!(artifact.library_digest.len(), 64, "{}", artifact.url);
            assert!(!artifact.member.is_empty());
            assert!(!artifact.filename.is_empty());

            for dependency in artifact.dependencies {
                check(dependency);
            }
        }

        for artifact in ALL {
            check(artifact);
        }
    }

    /// A dependency loaded from the wrong place would defeat the pinning, so
    /// they carry their own digests and are not shared between platforms.
    #[test]
    fn only_windows_arm64_has_a_dependency() {
        for artifact in ALL {
            let expected = std::ptr::eq(*artifact, &WINDOWS_ARM64);

            assert_eq!(
                !artifact.dependencies.is_empty(),
                expected,
                "{}",
                artifact.url
            );
        }

        assert_eq!(
            WINDOWS_ARM64.dependencies[0].filename,
            "libwinpthread-1.dll"
        );
    }

    #[test]
    fn only_windows_arm64_needs_patching() {
        for artifact in ALL {
            let expected =
                std::ptr::eq(*artifact, &WINDOWS_ARM64).then_some(Patch::WindowsArm64TcgMasks);

            assert_eq!(artifact.patch, expected, "{}", artifact.url);
        }
    }
}
