.PHONY: changelog changelog-preview release check lint

# Generate / update CHANGELOG.md
changelog:
	git-cliff --output CHANGELOG.md

# Preview unreleased changelog (does not write to file)
changelog-preview:
	git-cliff --unreleased

# Tag a new release. Pushing the tag starts the GitHub Release workflow.
# Usage: make release VERSION=0.1.0
release:
	@test -n "$(VERSION)" || (echo "Usage: make release VERSION=0.1.0" && exit 1)
	@test -z "$$(git status --short)" || (echo "Working tree must be clean before tagging a release." && exit 1)
	@grep -q '^version = "$(VERSION)"' Cargo.toml || (echo "Cargo.toml workspace version must be $(VERSION)." && exit 1)
	@grep -q '^## \[$(VERSION)\]' CHANGELOG.md || (echo "CHANGELOG.md must contain a $(VERSION) section." && exit 1)
	@echo "Releasing v$(VERSION)..."
	git tag -a v$(VERSION) -m "v$(VERSION)"
	@echo "Tagged v$(VERSION). Run 'git push origin HEAD --follow-tags' to publish."

# Run checks
check:
	cargo fmt --all --check
	cargo clippy --all-targets --all-features -- -D warnings

# Format code
lint:
	cargo fmt
