//! Pinned locations of prebuilt Unicorn libraries.
//!
//! Unicorn is not a build dependency: compiling its vendored QEMU needs a C
//! toolchain and breaks on newer compilers, and requiring a system libunicorn
//! would push that problem onto every user. Instead a prebuilt library is
//! fetched at runtime, exactly as the Apple assets are.
//!
//! The Python wheels on PyPI are the most convenient source on macOS and Linux
//! — a wheel is a zip archive containing a ready-built shared library for the
//! platform. Both the archive and the library it yields are pinned by digest.

use crate::error::ClientError;

pub const VERSION: &str = "2.1.4";

pub struct Artifact {
    pub url: &'static str,
    pub archive_digest: &'static str,
    pub library_digest: &'static str,
    /// Path of the shared library inside the archive.
    pub member: &'static str,
    /// Name the library is cached under.
    pub filename: &'static str,
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

    #[allow(unreachable_code)]
    Err(ClientError::Sap(format!(
        "no prebuilt Unicorn {VERSION} is configured for {}/{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )))
}

#[cfg(target_os = "macos")]
const DYLIB: &str = "unicorn/lib/libunicorn.2.dylib";
#[cfg(target_os = "linux")]
const SO: &str = "unicorn/lib/libunicorn.so.2";

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
static MACOS_X86_64: Artifact = Artifact {
    url: "https://files.pythonhosted.org/packages/c8/a7/92b47771e2107a201632a199cec91e8a81ee8a071ca6b7e7d600d8c61ac9/unicorn-2.1.4-cp37-abi3-macosx_10_9_x86_64.whl",
    archive_digest: "2a6f738fab5fabffa56af1e7bbf16ea1e91466c342f8dc64f125bd70f36c6b80",
    library_digest: "51c4a6f3ce22628ecd3acd1c49b921a818ffb989ca2c473134cc7eb06094f256",
    member: DYLIB,
    filename: "libunicorn.2.dylib",
};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
static MACOS_ARM64: Artifact = Artifact {
    url: "https://files.pythonhosted.org/packages/6c/ae/4943c6f8524d729ec7d5e69df6407ea05d710fe77471d91cecf3fc64eb57/unicorn-2.1.4-cp37-abi3-macosx_11_0_arm64.whl",
    archive_digest: "d6c93e0f60328d8f4a1792af3f834137a28050fcc2305f2ec01efe8558a9844e",
    library_digest: "7207c8e3d7a63118fb0bca73e01816797fd51b1d8a39a4cbc7abfd562ee59c85",
    member: DYLIB,
    filename: "libunicorn.2.dylib",
};

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
static LINUX_X86_64: Artifact = Artifact {
    url: "https://files.pythonhosted.org/packages/e7/df/ded5e3684c2d7600b30cc8a7530277b8cb36644a1a9d34cade7ebb45604c/unicorn-2.1.4-cp37-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl",
    archive_digest: "9d6e6dea140560de4ebd8446661f7ef84a357d428c14a3ef09dacd306ec8c239",
    library_digest: "ddb196ec82b52e502c18e4a34478bf7b9f61c83c2ebaa95c74d8ded45a95da9c",
    member: SO,
    filename: "libunicorn.so.2",
};

#[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
static LINUX_ARM64: Artifact = Artifact {
    url: "https://files.pythonhosted.org/packages/33/9f/32d41eb942221bcf4417cdc65537fc8b3bbbd6079d6c161e621f1dd4e94a/unicorn-2.1.4-cp37-abi3-manylinux_2_17_aarch64.manylinux2014_aarch64.whl",
    archive_digest: "bd1fb0c9af5f57e356d8a96928b4fe045b2e18f308ef23b481d5f970008aa722",
    library_digest: "a0b99458a82e268aee258205a40590411c3a9f28e42abf2942ce4e87b7d9ac65",
    member: SO,
    filename: "libunicorn.so.2",
};

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "musl"))]
static LINUX_MUSL_X86_64: Artifact = Artifact {
    url: "https://files.pythonhosted.org/packages/ed/4b/4628ccb20eb3ad1af400de8181d1f4e5c1a3fc2affa1b3410c1b2d71af36/unicorn-2.1.4-cp37-abi3-musllinux_1_2_x86_64.whl",
    archive_digest: "d348a90ee90219d141cb115ef8ed7e3fd1af42afaee105f7580761d775b25e32",
    library_digest: "cc1a208c69b151fdd23439736b0fac9ac6e14409dae77deee900369e6daab302",
    member: SO,
    filename: "libunicorn.so.2",
};

#[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "musl"))]
static LINUX_MUSL_ARM64: Artifact = Artifact {
    url: "https://files.pythonhosted.org/packages/70/38/ba5a051c844026e59ab6e0017db8cec77dbe20ab5f1d6edae1ce9d885b06/unicorn-2.1.4-cp37-abi3-musllinux_1_2_aarch64.whl",
    archive_digest: "01d744ba01c5cc68f1d7afe3d183f1868720fd440ec4eaedc4d1d5d9bf54b84c",
    library_digest: "52179305928b32c937d2d527ad6fef9d500c6fa7cdb14bf32abf7021d67271a2",
    member: SO,
    filename: "libunicorn.so.2",
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows needs 7z and zstd archives rather than a wheel, so it is not
    /// wired up yet; the error should say so rather than panic.
    #[test]
    fn resolves_or_explains() {
        match current() {
            Ok(artifact) => {
                assert!(artifact.url.starts_with("https://"));
                assert_eq!(artifact.archive_digest.len(), 64);
                assert_eq!(artifact.library_digest.len(), 64);
            }
            Err(e) => assert!(e.to_string().contains("no prebuilt Unicorn")),
        }
    }
}
