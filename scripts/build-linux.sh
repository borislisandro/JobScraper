#!/usr/bin/env bash
# Linux counterpart of build-app.ps1.
#   build-linux.sh dev       run the app against the Vite dev server
#   build-linux.sh package   release gates, then .rpm and .deb under artifacts/<version>/
set -euo pipefail

mode="${1:-dev}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo"

case "$(node --version)" in v24.*) ;; *) echo "Node 24 is required; found $(node --version)." >&2; exit 1 ;; esac
case "$(pnpm --version)" in 11.*) ;; *) echo "pnpm 11 is required; found $(pnpm --version)." >&2; exit 1 ;; esac
command -v cargo >/dev/null || { echo "Rust (cargo) is required." >&2; exit 1; }

pnpm install --frozen-lockfile
version="$(node -p "require('./package.json').version")"
[[ "$version" == "$(node -p "require('./src-tauri/tauri.conf.json').version")" ]] \
  || { echo "Version mismatch between package.json and tauri.conf.json." >&2; exit 1; }

case "$mode" in
  dev)
    scripts/prepare-release.sh
    exec pnpm tauri dev
    ;;
  package)
    echo "Running release gates..."
    pnpm check
    pnpm test
    # Builds src-tauri/sidecar, which the Tauri build script requires to exist even for `cargo test`.
    scripts/prepare-release.sh
    cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
    cargo test --manifest-path src-tauri/Cargo.toml
    pnpm tauri build --bundles deb,rpm --ci
    out="artifacts/$version"
    mkdir -p "$out"
    find src-tauri/target/release/bundle \( -name '*.rpm' -o -name '*.deb' \) -exec cp -f {} "$out/" \;
    (cd "$out" && sha256sum -- *.rpm *.deb > SHA256SUMS.txt && sha256sum -c SHA256SUMS.txt)
    echo "Packaging complete: $repo/$out"
    ;;
  *)
    echo "Usage: build-linux.sh dev|package" >&2
    exit 2
    ;;
esac
