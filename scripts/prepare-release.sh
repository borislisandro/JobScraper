#!/usr/bin/env bash
# Linux counterpart of prepare-release.ps1. With --preflight-only it only checks the prepared
# sidecar; otherwise it builds the sidecar first.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "${1:-}" != "--preflight-only" ]]; then
  "$repo/scripts/package-sidecar.sh" "${JOBSCRAPER_NODE_RUNTIME:-}" "${JOBSCRAPER_PNPM:-}"
fi

sidecar="$repo/src-tauri/sidecar"
for path in node worker.mjs adapters.mjs company-adapters.mjs request-policy.mjs browser-proxy.mjs browser-path.mjs \
  node_modules/playwright-core/package.json node_modules/cheerio/package.json \
  node_modules/fast-xml-parser/package.json node_modules/xpath/package.json node_modules/@xmldom/xmldom/package.json; do
  [[ -f "$sidecar/$path" ]] || { echo "Missing packaged sidecar resource: $path" >&2; exit 1; }
done

version="$("$sidecar/node" --version)"
[[ "$version" == v24.* ]] || { echo "Bundled Node must be 24 LTS; found $version" >&2; exit 1; }
(cd "$sidecar" && ./node --input-type=module -e "await import('./worker.mjs'); await import('./adapters.mjs');") \
  || { echo "Bundled worker module smoke failed." >&2; exit 1; }
echo "Release preflight passed: Node $version; bundled sidecar modules load."
