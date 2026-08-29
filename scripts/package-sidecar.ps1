param(
  [Parameter(Mandatory=$true)][string]$NodePath,
  [Parameter(Mandatory=$true)][string]$PnpmPath
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$source = Join-Path $repo 'sidecar'
$dest = [IO.Path]::GetFullPath((Join-Path $repo 'src-tauri\sidecar'))
$expectedDest = [IO.Path]::GetFullPath((Join-Path $repo 'src-tauri\sidecar'))
if ($dest -ne $expectedDest -or -not $dest.StartsWith($repo, [StringComparison]::OrdinalIgnoreCase)) {
  throw 'Refusing to replace a sidecar directory outside the repository release path.'
}
$node = (Resolve-Path $NodePath).Path
$pnpm = (Resolve-Path $PnpmPath).Path
if (Test-Path $dest) { Remove-Item -LiteralPath $dest -Recurse -Force }
if (-not (Test-Path (Join-Path $repo 'pnpm-lock.yaml'))) { throw 'Missing pinned workspace pnpm-lock.yaml.' }
# NSIS copies files but does not recreate pnpm junctions. Hoisted mode emits a
# physical production tree that remains resolvable after installation.
$pnpmArgs = @('--config.node-linker=hoisted', '--dir', $repo, '--filter', 'jobscraper-sidecar', 'deploy', '--prod', $dest)
if ([IO.Path]::GetExtension($pnpm) -in @('.js', '.cjs', '.mjs')) {
  & $node $pnpm @pnpmArgs
}
else {
  & $pnpm @pnpmArgs
}
if ($LASTEXITCODE -ne 0) { throw 'Sidecar production dependency deployment failed.' }
Copy-Item -LiteralPath $node -Destination (Join-Path $dest 'node.exe')
Write-Host "Prepared isolated Node 24 sidecar at $dest"
