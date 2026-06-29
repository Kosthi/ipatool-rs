#!/usr/bin/env bash
set -euo pipefail

target="${1:?target is required}"
version="${2:?version is required}"

binary="target/${target}/release/ipatool"
dist_dir="dist"
asset_stem="ipatool-${version}-${target}"
stage_dir="target/package/${asset_stem}"

if [[ ! -x "$binary" ]]; then
  echo "missing release binary: $binary" >&2
  exit 1
fi

rm -rf "$stage_dir"
mkdir -p "$stage_dir"
mkdir -p "$dist_dir"

cp "$binary" "${stage_dir}/ipatool"
cp LICENSE "${stage_dir}/LICENSE"
chmod 0755 "${stage_dir}/ipatool"

cat > "${stage_dir}/README.txt" <<EOF
ipatool ${version}

This package contains the ipatool command-line binary for ${target}.

Install:
  sudo ./install.sh

Verify:
  ipatool --help

The default install prefix is /usr/local. Override it with:
  sudo PREFIX=/opt/homebrew ./install.sh
EOF

cat > "${stage_dir}/install.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
prefix="${PREFIX:-/usr/local}"
install_dir="${prefix}/bin"

install -d "$install_dir"
install -m 0755 "${script_dir}/ipatool" "${install_dir}/ipatool"

echo "Installed ipatool to ${install_dir}/ipatool"
EOF
chmod 0755 "${stage_dir}/install.sh"

tar -C "$(dirname "$stage_dir")" -czf "${dist_dir}/${asset_stem}.tar.gz" "$asset_stem"

if [[ "$target" == *-apple-darwin ]]; then
  hdiutil create \
    -volname "ipatool ${version}" \
    -srcfolder "$stage_dir" \
    -ov \
    -format UDZO \
    "${dist_dir}/${asset_stem}.dmg"
fi
