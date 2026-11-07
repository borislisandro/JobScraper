#!/usr/bin/env bash
# Linux counterpart of package-sidecar.ps1: builds the isolated production tree of the Node worker
# at src-tauri/sidecar, with the Node 24 runtime beside it as `node`.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
node="${1:-${JOBSCRAPER_NODE_RUNTIME:-$(command -v node || true)}}"
pnpm="${2:-${JOBSCRAPER_PNPM:-$(command -v pnpm || true)}}"
dest="$repo/src-tauri/sidecar"

[[ -x "$node" ]] || { echo "Node runtime not found; pass its path or set JOBSCRAPER_NODE_RUNTIME." >&2; exit 1; }
[[ -x "$pnpm" ]] || { echo "pnpm not found; pass its path or set JOBSCRAPER_PNPM." >&2; exit 1; }
[[ -f "$repo/pnpm-lock.yaml" ]] || { echo "Missing pinned workspace pnpm-lock.yaml." >&2; exit 1; }
case "$("$node" --version)" in v24.*) ;; *) echo "Node 24 is required; found $("$node" --version)." >&2; exit 1 ;; esac

# Only ever replace the release path inside this repository.
[[ "$dest" == "$repo/src-tauri/sidecar" ]] || { echo "Refusing to replace a sidecar directory outside the repository." >&2; exit 1; }
rm -rf "$dest"

# Packages are copied, not linked, so hoisted mode gives a physical tree that still resolves once
# it is installed somewhere else. PATH carries the right node for pnpm's own scripts.
PATH="$(dirname "$node"):$PATH" "$pnpm" --config.node-linker=hoisted --dir "$repo" --filter jobscraper-sidecar deploy --prod "$dest"
install -m 0755 "$node" "$dest/node"
echo "Prepared isolated Node 24 sidecar at $dest"
