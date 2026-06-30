# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Stop persisting the raw Apple ID password in stored credentials and redact it from account JSON output.
- Limit verbose logging to `ipatool` crates and remove App Store response body previews from debug logs.
- Create the `~/.ipatool` data directory and cookie file with private permissions on Unix platforms.

## [0.1.3] - 2026-06-30

### Fixed

- Read `version meta` values from the selected IPA's main `Info.plist` instead of stale App Store download metadata.
- Added targeted remote ZIP range reading for `Info.plist` so version metadata can be inspected without downloading the full IPA.
- Re-authenticate and retry `version list` and `version meta` when stored password tokens expire.

## [0.1.2] - 2026-06-29

### Fixed

- Fixed `version list` requests for Apple's current response shape, including parsing version IDs from `songList` metadata and exposing the latest external version ID.
- Improved `version list` text output by removing placeholder `version: ?` labels and marking the latest external version.
- Added clearer guidance when version commands require purchasing the app first.
- Preserved the selected IPA app bundle and directory metadata when patching downloaded archives.

## [0.1.1] - 2026-06-29

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
