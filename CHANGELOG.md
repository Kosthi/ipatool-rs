# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Migrated release packaging to `cargo-dist`, producing Homebrew, shell, and PowerShell installers plus platform archives and checksums.
- Replaced macOS DMG packaging with CLI-friendly `tar.xz` archives to avoid unsigned installer script Gatekeeper prompts.

## [0.1.0] - 2026-06-29

### Added

- Initial Rust implementation of the `ipatool` CLI for searching, purchasing, and downloading IPA files.
- Interactive terminal UI with Search, Library, Downloads, and Account tabs.
- Apple ID login, 2FA handling, credential storage, and token refresh flows.
- App Store search, purchase, download, version listing, and IPA patching commands.
- Text and JSON output modes for scripting.
- CI and release workflows for Linux, macOS, and Windows builds.
