param([switch]$FetchModel, [switch]$PreflightOnly)
$ErrorActionPreference = 'Stop'
function Get-Sha256([string]$Path) {
  $stream = [IO.File]::OpenRead($Path)
  try {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return -join ($sha.ComputeHash($stream) | ForEach-Object { $_.ToString('x2') }) }
    finally { $sha.Dispose() }
  }
  finally { $stream.Dispose() }
}
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$model = Join-Path $repo 'src-tauri\resources\models\bge-small-en-v1.5'
$manifestPath = Join-Path $repo 'scripts\bge-small-en-v1.5.manifest.json'
if ($FetchModel) { & (Join-Path $PSScriptRoot 'fetch-bge-model.ps1') -Destination $model }
if (-not $PreflightOnly) {
  if (-not $env:JOBSCRAPER_NODE_RUNTIME -or -not $env:JOBSCRAPER_PNPM) { throw 'Set JOBSCRAPER_NODE_RUNTIME and JOBSCRAPER_PNPM; release packaging never uses a system Node or pnpm.' }
  & (Join-Path $PSScriptRoot 'package-sidecar.ps1') -NodePath $env:JOBSCRAPER_NODE_RUNTIME -PnpmPath $env:JOBSCRAPER_PNPM
}
if (-not (Test-Path $manifestPath)) { throw 'Missing pinned BGE manifest.' }
$manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
foreach ($entry in $manifest.files) {
  $path = Join-Path $model $entry.path
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing packaged BGE file: $($entry.path)" }
  $info = Get-Item -LiteralPath $path
  if ($info.Length -ne [int64]$entry.size) { throw "BGE size mismatch: $($entry.path)" }
  if ((Get-Sha256 $path) -ne $entry.sha256) { throw "BGE checksum mismatch: $($entry.path)" }
}
$sidecar = Join-Path $repo 'src-tauri\sidecar'
foreach ($path in @('node.exe','worker.mjs','adapters.mjs','node_modules\playwright-core\package.json','node_modules\cheerio\package.json','node_modules\fast-xml-parser\package.json','node_modules\xpath\package.json','node_modules\@xmldom\xmldom\package.json')) {
  if (-not (Test-Path -LiteralPath (Join-Path $sidecar $path) -PathType Leaf)) { throw "Missing packaged sidecar resource: $path" }
}
$nodeVersion = & (Join-Path $sidecar 'node.exe') --version
if ($LASTEXITCODE -ne 0 -or $nodeVersion -notmatch '^v24\.') { throw "Bundled Node must be 24 LTS; found $nodeVersion" }
Push-Location $sidecar
try { & .\node.exe --input-type=module -e "await import('./worker.mjs'); await import('./adapters.mjs');" } finally { Pop-Location }
if ($LASTEXITCODE -ne 0) { throw 'Bundled worker module smoke failed.' }
Write-Host "Release preflight passed: Node $nodeVersion; pinned offline BGE manifest valid."
