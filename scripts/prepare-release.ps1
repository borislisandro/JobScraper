param(
  [switch]$PreflightOnly,
  [string]$NodePath,
  [string]$PnpmPath
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $PreflightOnly) {
  if (-not $NodePath) { $NodePath = $env:JOBSCRAPER_NODE_RUNTIME }
  if (-not $PnpmPath) { $PnpmPath = $env:JOBSCRAPER_PNPM }
  if (-not $NodePath -or -not $PnpmPath) { throw 'Pass -NodePath and -PnpmPath, or set JOBSCRAPER_NODE_RUNTIME and JOBSCRAPER_PNPM.' }
  & (Join-Path $PSScriptRoot 'package-sidecar.ps1') -NodePath $NodePath -PnpmPath $PnpmPath
}
$sidecar = Join-Path $repo 'src-tauri\sidecar'
foreach ($path in @('node.exe','worker.mjs','adapters.mjs','company-adapters.mjs','node_modules\playwright-core\package.json','node_modules\cheerio\package.json','node_modules\fast-xml-parser\package.json','node_modules\xpath\package.json','node_modules\@xmldom\xmldom\package.json')) {
  if (-not (Test-Path -LiteralPath (Join-Path $sidecar $path) -PathType Leaf)) { throw "Missing packaged sidecar resource: $path" }
}
$nodeVersion = & (Join-Path $sidecar 'node.exe') --version
if ($LASTEXITCODE -ne 0 -or $nodeVersion -notmatch '^v24\.') { throw "Bundled Node must be 24 LTS; found $nodeVersion" }
Push-Location $sidecar
try { & .\node.exe --input-type=module -e "await import('./worker.mjs'); await import('./adapters.mjs');" } finally { Pop-Location }
if ($LASTEXITCODE -ne 0) { throw 'Bundled worker module smoke failed.' }
Write-Host "Release preflight passed: Node $nodeVersion; bundled sidecar modules load."
