# Contributing to ipatool-rs

Thank you for your interest in contributing. This document covers how to build the project, run checks, and submit changes.

## Prerequisites

- Rust (stable, 2024 edition) — install via [rustup](https://rustup.rs/)
- macOS, Linux, or Windows

## Build

```bash
git clone https://github.com/Kosthi/ipatool-rs.git
cd ipatool-rs
cargo build
```

The binary lands at `target/debug/ipatool`. For a release build: `cargo build --release`.

## Checks

Before opening a PR, run the full check suite:

```bash
make check   # cargo fmt --check + cargo clippy -D warnings
```

To auto-format: `make lint`

## Project layout

The workspace has two crates:

- **`crates/ipatool-core`** — all Apple API logic (auth, search, purchase, download, IPA patching). No CLI concerns here.
- **`crates/ipatool-cli`** — the binary: clap command parsing, TUI, output formatters. Depends on `ipatool-core`.

Keep API/protocol logic in `ipatool-core` and presentation logic in `ipatool-cli`.

## Submitting a pull request

1. Fork the repo and create a branch from `master`.
2. Keep commits [Conventional Commits](https://www.conventionalcommits.org/) format (`feat:`, `fix:`, `refactor:`, etc.) — the changelog is generated from these.
3. Run `make check` and ensure it passes.
4. Open a PR against `master`. Describe what you changed and why.

## Bug reports

Use the [bug report template](.github/ISSUE_TEMPLATE/bug_report.yml). Always include `--verbose` output (redact your Apple ID email first).

## Questions

Open a [GitHub Discussion](https://github.com/Kosthi/ipatool-rs/discussions) for usage questions or ideas that aren't ready to be issues.
