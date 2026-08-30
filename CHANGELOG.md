# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.8] - 2026-08-30

### ⚙️ Miscellaneous

- **ci:** publish to crates.io from the release workflow
## [0.1.7] - 2026-08-30

### 🚀 Features

- **core:** redirect Unicorn's Windows imports so the DLL can be called
- **core:** pin and extract Unicorn for Windows
- **core:** restore sign-in with SAP request signing
### 🐛 Bug Fixes

- **core:** satisfy the Windows lints on the import patcher
- **core:** read randomness portably in the guest shim
### 📚 Documentation

- explain what the first sign-in downloads
### ⚙️ Miscellaneous

- run the live tests weekly
## [0.1.6] - 2026-08-18

### 🐛 Bug Fixes

- **ci:** refresh release lockfile
- **core:** keep POST body across auth pod redirects
### ⚙️ Miscellaneous

- add crates metadata and community docs
## [0.1.5] - 2026-07-02

### 🐛 Bug Fixes

- **cli:** read visible prompts from terminal
- **cli:** improve login input UX
## [0.1.4] - 2026-07-02

### 🐛 Bug Fixes

- **cli:** clarify version download output
- split Windows console access
- keep Windows password prompt visible
- handle Windows input duplication
## [0.1.3] - 2026-06-30

### 🐛 Bug Fixes

- avoid duplicate reauth logs
- read version metadata from IPA plist
### ⚙️ Miscellaneous

- release v0.1.3
## [0.1.2] - 2026-06-29

### 🐛 Bug Fixes

- **core:** preserve IPA app selection and directory metadata
- repair version list parsing
### 📚 Documentation

- update 0.1.2 changelog
## [0.1.1] - 2026-06-29

### 🐛 Bug Fixes

- **cli:** handle 2fa login rejection
### 📚 Documentation

- document ipa output path
### ⚙️ Miscellaneous

- release v0.1.1
## [0.1.0] - 2026-06-29

### 🚀 Features

- add TUI mode and improve token/retry resilience
### 🐛 Bug Fixes

- align App Store download license handling
- resolve clippy and rustfmt warnings in TUI code
- resolve rustfmt and clippy warnings
- correct repository URL in cliff.toml
### 📚 Documentation

- let logo lead README
- showcase TUI in README
### 🎨 Style

- reformat code for Rust 1.96 rustfmt
### ⚙️ Miscellaneous

- migrate releases to cargo-dist
- add installable release artifacts
- add CI and release workflows
- add commit convention and changelog tooling
[0.1.7]: https://github.com/Kosthi/ipatool-rs/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/Kosthi/ipatool-rs/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/Kosthi/ipatool-rs/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/Kosthi/ipatool-rs/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/Kosthi/ipatool-rs/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/Kosthi/ipatool-rs/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/Kosthi/ipatool-rs/compare/v0.1.0...v0.1.1

