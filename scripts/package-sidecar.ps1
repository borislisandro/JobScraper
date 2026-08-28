param(
  [Parameter(Mandatory=$true)][string]$NodePath
)
$ErrorActionPreference = 'Stop'
$dest = Join-Path $PSScriptRoot '..\src-tauri\sidecar'
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item -LiteralPath $NodePath -Destination (Join-Path $dest 'node.exe') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\sidecar\worker.mjs') -Destination $dest -Force
Write-Host "Bundled Node 24 worker in $dest. Deploy sidecar production dependencies before packaging."
