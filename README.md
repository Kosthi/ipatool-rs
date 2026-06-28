# ipatool-rs

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024_edition-orange.svg)](https://www.rust-lang.org/)

A working, open-source command-line tool to search, purchase, and download iOS App Store packages (IPA files).

**Rewritten in Rust** from [majd/ipatool](https://github.com/majd/ipatool). As of June 2026, Apple has changed its authentication endpoints multiple times, breaking the original Go implementation and its many forks. **ipatool-rs is currently the only open-source ipatool that works out of the box.**

## Why a rewrite?

| | Go (original) | Rust (this project) |
|---|---|---|
| Auth | Broken (Apple endpoint changes) | Working (adapted to latest Apple auth flow) |
| Error messages | "something went wrong" | Structured errors with clear failure reasons |
| Type safety | Runtime plist field access | Compile-time typed models with serde |
| Binary size | ~15 MB (with runtime) | ~7 MB static binary, no runtime |
| Memory | GC pauses on large downloads | Zero-copy streaming, no GC |

## Requirements

- macOS / Linux / Windows
- An Apple ID with App Store access

## Installation

### Build from source

```bash
git clone https://github.com/Kosthi/ipatool-rs.git
cd ipatool-rs
cargo build --release
# Binary at target/release/ipatool
```

## Usage

### Auth

Log in with your Apple ID. Supports two-factor authentication.

```bash
# Interactive login
ipatool auth login --email your@apple.id --password 'password'

# With 2FA code (run after first login prompts for 2FA)
ipatool auth login --email your@apple.id --password 'password' --auth-code 123456

# Show current account
ipatool auth info

# Log out and clear credentials
ipatool auth revoke
```

### Search

Search for apps on the App Store.

```bash
ipatool search "WeChat" --limit 10
ipatool search "Telegram" --limit 5 --country US
```

### Purchase

Obtain a free license for an app (required before downloading).

```bash
ipatool purchase -b com.tencent.xin
```

### Download

Download an IPA file. Use `--purchase` to automatically obtain the license.

```bash
# Download with auto-purchase
ipatool download -b com.tencent.xin --purchase

# Specify output path
ipatool download -b com.tencent.xin --purchase -o wechat.ipa

# Download by app ID
ipatool download -i 414478124 --purchase

# Download a specific version
ipatool download -b com.tencent.xin --version-id 12345
```

### Version

List available versions and retrieve version metadata.

```bash
# List all versions
ipatool version list -b com.tencent.xin

# Get metadata for a specific version
ipatool version meta -b com.tencent.xin --version-id 12345
```

### Global Flags

```
--format <text|json>    Output format (default: text)
--verbose               Enable debug logging
--non-interactive       Disable interactive prompts
```

## Project Structure

```
ipatool-rs/
├── Cargo.toml                  # Workspace root
└── crates/
    ├── ipatool-core/           # Core library (reusable)
    │   └── src/
    │       ├── api/            # Apple API endpoints (auth, search, purchase, download)
    │       ├── client/         # HTTP client, plist parser, cookie jar
    │       ├── model/          # Account, App, Platform, StoreFront types
    │       ├── ipa/            # IPA patching (SINF + metadata injection)
    │       ├── error.rs        # Three-layer error hierarchy
    │       ├── credential.rs   # Keychain storage
    │       └── guid.rs         # Device GUID generation
    └── ipatool-cli/            # CLI binary
        └── src/
            ├── main.rs         # Entry point + clap arg parsing
            ├── output.rs       # Text/JSON formatters
            └── commands/       # Subcommand handlers
```

## How it works

1. **Auth** — Posts credentials to Apple's native auth endpoint (`auth.itunes.apple.com/auth/v1/native/fast/`) using the legacy MZFinance protocol. Handles 2FA, redirects, and retry logic. Stores the session token in the system keychain and cookies on disk.

2. **Search/Lookup** — Queries the public iTunes Search API (`itunes.apple.com/search`).

3. **Purchase** — Sends a buy request to `buy.itunes.apple.com` with STDQ pricing (falls back to GAME for Apple Arcade). Tolerates "license already exists".

4. **Download** — Fetches download URL and DRM data (SINF) from Apple's `volumeStoreDownloadProduct` endpoint. Streams the IPA with progress display and HTTP Range resume support.

5. **Patch** — Rebuilds the ZIP: injects `iTunesMetadata.plist` (purchase metadata + Apple ID) and SINF files (DRM authorization) into the IPA. Without this step, the IPA cannot be installed on a device.

## License

[MIT](LICENSE)
