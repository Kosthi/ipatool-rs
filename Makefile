.PHONY: changelog changelog-preview release check lint

# Generate / update CHANGELOG.md
changelog:
	git-cliff --output CHANGELOG.md

# Preview unreleased changelog (does not write to file)
changelog-preview:
	git-cliff --unreleased

# Tag a new release (usage: make release VERSION=0.2.0)
release:
	@test -n "$(VERSION)" || (echo "Usage: make release VERSION=0.2.0" && exit 1)
	@echo "Releasing v$(VERSION)..."
	cargo set-version $(VERSION)
	git-cliff --tag v$(VERSION) --output CHANGELOG.md
	git add -A
	git commit -m "chore(release): v$(VERSION)"
	git tag -a v$(VERSION) -m "v$(VERSION)"
	@echo "Tagged v$(VERSION). Run 'git push --follow-tags' to publish."

# Run checks
check:
	cargo fmt --check
	cargo clippy -- -D warnings

# Format code
lint:
	cargo fmt
