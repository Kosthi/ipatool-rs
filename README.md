<p align="center">
  <img src="media/logo.svg" alt="ipatool-rs" width="620">
</p>

<p align="center">
  A terminal UI and CLI for searching, purchasing, and downloading iOS App Store IPA files. Like ipatool, but rebuilt in Rust and adapted to Apple's current auth flow.
</p>

<p align="center">
  <a href="https://github.com/Kosthi/ipatool-rs/actions/workflows/ci.yml"><img src="https://github.com/Kosthi/ipatool-rs/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-2024_edition-orange.svg" alt="Rust 2024"></a>
  <a href="https://opensource.org/licenses/MIT"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"></a>
  <a href="https://star-history.com/#Kosthi/ipatool-rs&Date"><img src="https://img.shields.io/github/stars/Kosthi/ipatool-rs?style=flat&label=Stars" alt="GitHub Stars"></a>
</p>

<p align="center">
  <a href="media/demo.svg"><img src="media/demo.svg" alt="ipatool-rs TUI demo" width="860"></a>
</p>

[English](README.md) | [中文](README_zh.md)

## Why This Exists

The original Go [ipatool](https://github.com/majd/ipatool) and many forks broke after Apple changed authentication endpoints.
`ipatool-rs` keeps the same practical goal: log in with an Apple ID, search the App Store, obtain free licenses, and download IPA files.

This rewrite adds a keyboard-driven terminal UI, structured Rust models, clearer errors, streaming downloads, and retry/reauth flows for unstable App Store responses.

## Comparison

| Feature | [ipatool](https://github.com/majd/ipatool) (Go) | ipatool-rs (this project) |
|---------|--------------------------------------------------|---------------------------|
| Apple ID authentication | ❌ Broken since 2024 auth changes | ✅ Works with current endpoints |
| Two-factor authentication | ❌ Broken | ✅ Supported |
| Interactive TUI | ❌ CLI only | ✅ Full keyboard-driven TUI |
| Download progress | ❌ None | ✅ Progress bar + streaming |
| HTTP Range resume | ❌ No | ✅ Yes (CLI mode) |
| Background download queue | ❌ No | ✅ TUI downloads tab |
| Session auto-refresh | ❌ No | ✅ Re-auths on expired token |
| Credential storage | File-based | ✅ OS keychain (macOS/Linux/Windows) |
| Output formats | text | text + JSON |
| Single binary | ✅ | ✅ |
| Windows support | ✅ | ✅ |

## Features

- Interactive TUI mode: run `ipatool` to open tabs for Search, Library, Downloads, and Account.
- Search-to-download workflow: browse App Store results, inspect app details, purchase free licenses, and queue downloads from one screen.
- Download dashboard: track progress, failures, cancellation, and completed items in the Downloads tab.
- Account management: log in, handle 2FA, view the active account, and revoke stored credentials.
- Resilient sessions: refresh expired tokens during purchase and download flows when stored credentials are available.
- Robust downloads: stream IPA files with progress display and HTTP Range resume support in CLI mode.
- Patch-ready IPAs: inject purchase metadata and SINF authorization data into the downloaded archive.
- Text or JSON output for scripts and automation.

## Installation

### Homebrew (macOS / Linux)

```bash
brew install Kosthi/tap/ipatool
```

### Cargo

```bash
cargo install ipatool-rs
```

### Prebuilt Release Assets

Download the latest files from the [GitHub Releases](https://github.com/Kosthi/ipatool-rs/releases) page.

The release also includes `cargo-dist` installer scripts that install the `ipatool` binary into your Cargo bin directory.

```bash
# macOS / Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Kosthi/ipatool-rs/releases/latest/download/ipatool-installer.sh | sh

# Windows PowerShell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/Kosthi/ipatool-rs/releases/latest/download/ipatool-installer.ps1 | iex"
```

| Platform | Asset | Install |
|----------|-------|---------|
| Windows x64 | `ipatool-x86_64-pc-windows-msvc.zip` | Extract and place `ipatool.exe` on your `PATH`. |
| macOS Apple Silicon | `ipatool-aarch64-apple-darwin.tar.xz` | Extract and place `ipatool` on your `PATH`. |
| macOS Intel | `ipatool-x86_64-apple-darwin.tar.xz` | Extract and place `ipatool` on your `PATH`. |
| Linux x64 | `ipatool-x86_64-unknown-linux-gnu.tar.xz` | Extract and place `ipatool` on your `PATH`. |
| Linux ARM64 | `ipatool-aarch64-unknown-linux-gnu.tar.xz` | Extract and place `ipatool` on your `PATH`. |

Each release also includes per-asset `.sha256` files and a unified `sha256.sum`.

### Build From Source

```bash
git clone https://github.com/Kosthi/ipatool-rs.git
cd ipatool-rs
cargo build --release

# Binary at target/release/ipatool
```

## Usage

### Terminal UI

Launch the TUI by running `ipatool` without a subcommand.

```bash
ipatool
```

The UI is built with [ratatui](https://ratatui.rs/) and [crossterm](https://github.com/crossterm-rs/crossterm). It provides a fast keyboard workflow for login, search, purchase, download progress, and cancellation.

### CLI

Log in with your Apple ID. Two-factor authentication is supported.

```bash
# Interactive login (prompts for a 2FA code if Apple requires one)
ipatool auth login --email your@apple.id --password 'password'

# With 2FA code
ipatool auth login --email your@apple.id --password 'password' --auth-code 123456

# Show current account
ipatool auth info

# Log out and clear credentials
ipatool auth revoke
```

Search for apps on the App Store.

```bash
ipatool search "WeChat" --limit 10
ipatool search "Telegram" --limit 5 --country US
```

Obtain a free license for an app.

```bash
ipatool purchase -b com.tencent.xin
```

Download an IPA file. Use `--purchase` to automatically obtain the license before downloading.
By default, the IPA is saved in the current working directory as `{bundle_id}_{app_id}_{version}.ipa`.
Use `-o` to choose a different output path.

```bash
# Download with auto-purchase
ipatool download -b com.tencent.xin --purchase

# Specify output path
ipatool download -b com.tencent.xin --purchase -o ~/Downloads/wechat.ipa

# Download by app ID
ipatool download -i 414478124 --purchase

# Download a specific version
ipatool download -b com.tencent.xin --version-id 12345
```

List available versions and retrieve version metadata.

```bash
# List all versions
ipatool version list -b com.tencent.xin

# Get metadata for a specific version
ipatool version meta -b com.tencent.xin --version-id 12345
```

### Global Flags

```text
--format <text|json>    Output format (default: text)
--verbose               Enable debug logging
--non-interactive       Disable interactive prompts
```

## Keyboard Controls

**Global**

| Key | Action |
|-----|--------|
| `q` / `Ctrl+C` | Quit |
| `Tab` / `Shift+Tab` | Switch tabs |
| `1`-`4` | Jump to a tab |
| `j/k` or `Up/Down` | Navigate the current list |
| `Esc` | Cancel input or close a popup |

**Search tab**

| Key | Action |
|-----|--------|
| `/` / `s` | Focus the search input |
| `Enter` | Run search |
| `d` | Download selected app |
| `p` | Purchase selected app |

**Downloads tab**

| Key | Action |
|-----|--------|
| `x` | Cancel selected download |
| `c` | Clear finished downloads |

**Account tab**

| Key | Action |
|-----|--------|
| `l` | Log in |
| `r` | Revoke stored credentials |

## Project Structure

```text
ipatool-rs/
|-- Cargo.toml                  # Workspace root
|-- media/                      # README logo and demo assets
`-- crates/
    |-- ipatool-core/           # Core library
    |   `-- src/
    |       |-- api/            # Apple API endpoints
    |       |-- client/         # HTTP client, plist parser, cookie jar
    |       |-- model/          # Account, App, Platform, StoreFront types
    |       |-- ipa/            # IPA patching
    |       |-- error.rs        # Error hierarchy
    |       |-- credential.rs   # Keychain storage
    |       `-- guid.rs         # Device GUID generation
    `-- ipatool-cli/            # CLI binary
        `-- src/
            |-- main.rs         # Entry point and clap parsing
            |-- output.rs       # Text/JSON formatters
            |-- tui/            # Interactive terminal UI
            `-- commands/       # Subcommand handlers
```

## How It Works

1. **Auth** - Posts credentials to Apple's native auth endpoint using the legacy MZFinance protocol. Handles 2FA, redirects, retry logic, keychain storage, and cookies.

2. **Search/Lookup** - Queries the public iTunes Search API for app metadata and storefront-specific results.

3. **Purchase** - Sends a buy request to `buy.itunes.apple.com` with STDQ pricing and falls back to GAME for Apple Arcade-style responses.

4. **Download** - Fetches the download URL and DRM data from Apple's `volumeStoreDownloadProduct` endpoint, then streams the IPA.

5. **Patch** - Rebuilds the ZIP, injecting `iTunesMetadata.plist` and SINF files so the IPA can be installed on an authorized device.

6. **TUI** - Wraps the same core APIs in an async ratatui/crossterm interface with background tasks for search, login, purchase, and downloads.

## FAQ

**Is my Apple ID password stored on disk?**

No. After a successful login, ipatool-rs stores only a `password_token` (a session token issued by Apple) plus non-sensitive account metadata. The token is saved in the OS keychain — macOS Keychain, libsecret on Linux, or Windows Credential Manager — not in a plain-text file. Your actual password is never persisted.

**Why does ipatool-rs store the password at all for re-authentication?**

Some Apple Store operations (e.g. re-purchasing after a session expiry) require a full re-login, not just a token refresh. If you logged in interactively, ipatool-rs will prompt you again rather than storing the plain-text password. The `password` field in the keychain entry is only populated when you explicitly provide it via `--password` on the CLI, so re-auth can happen unattended in scripts.

**Does this violate Apple's Terms of Service?**

Downloading apps you have legitimately purchased or obtained a free license for is the same as using the App Store. This tool does not bypass DRM or enable piracy — downloaded IPAs still contain your account's DRM data and will only install on devices authorized under your Apple ID. Review Apple's terms for your specific use case.

**The original ipatool stopped working — is this a drop-in replacement?**

The CLI commands are intentionally similar, but not identical. The main differences are: `auth login` instead of `ipatool auth`, the addition of `--purchase` on download, and the new TUI mode. See the Usage section above for the full reference.

**I'm getting a 2FA prompt every time I run a command.**

This usually means the stored token has expired and Apple is requiring a fresh login. Run `ipatool auth login` again. If you're running in a script, use `--non-interactive` and ensure your session is fresh before the run.

**Which countries/storefronts are supported?**

All storefronts supported by the iTunes Search API. Use `--country <ISO-3166-1-alpha-2>` (e.g. `--country US`, `--country CN`) to target a specific store.

---

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=Kosthi/ipatool-rs&type=Date)](https://star-history.com/#Kosthi/ipatool-rs&Date)

## License

[MIT](LICENSE)
