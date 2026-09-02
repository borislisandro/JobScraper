param(
  [ValidateSet('Dev', 'Installer', 'Portable', 'All')]
  [string]$Mode = 'Dev',
  [string]$NodePath,
  [string]$PnpmPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$tauriDir = Join-Path $repo 'src-tauri'
$cargoManifest = Join-Path $tauriDir 'Cargo.toml'
$sidecarSource = Join-Path $tauriDir 'sidecar'

function Resolve-ToolPath {
  param(
    [string]$ExplicitPath,
    [string]$EnvironmentVariable,
    [string]$CommandName
  )

  $candidate = $ExplicitPath
  if (-not $candidate -and $EnvironmentVariable) {
    $candidate = [Environment]::GetEnvironmentVariable($EnvironmentVariable)
  }
  if (-not $candidate) { $candidate = $CommandName }

  if (Test-Path -LiteralPath $candidate -PathType Leaf) {
    return (Resolve-Path -LiteralPath $candidate).Path
  }

  $command = Get-Command $candidate -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $command) { throw "Required tool was not found: $CommandName" }
  if ($command.Path) { return $command.Path }
  return $command.Source
}

function Invoke-Native {
  param(
    [string]$FilePath,
    [string[]]$CommandArguments
  )

  & $FilePath @CommandArguments
  if ($LASTEXITCODE -ne 0) {
    throw "Command failed with exit code $LASTEXITCODE`: $FilePath $($CommandArguments -join ' ')"
  }
}

function Get-NativeOutput {
  param(
    [string]$FilePath,
    [string[]]$CommandArguments
  )

  $output = & $FilePath @CommandArguments
  if ($LASTEXITCODE -ne 0) {
    throw "Command failed with exit code $LASTEXITCODE`: $FilePath $($CommandArguments -join ' ')`n$($output | Out-String)"
  }
  return ($output | Out-String).Trim()
}

function Invoke-Pnpm {
  param([string[]]$CommandArguments)

  if ($script:pnpmIsNodeScript) {
    Invoke-Native -FilePath $script:resolvedNode -CommandArguments (@($script:resolvedPnpm) + $CommandArguments)
  } else {
    Invoke-Native -FilePath $script:resolvedPnpm -CommandArguments $CommandArguments
  }
}

function Get-PnpmOutput {
  param([string[]]$CommandArguments)

  if ($script:pnpmIsNodeScript) {
    return Get-NativeOutput -FilePath $script:resolvedNode -CommandArguments (@($script:resolvedPnpm) + $CommandArguments)
  }
  return Get-NativeOutput -FilePath $script:resolvedPnpm -CommandArguments $CommandArguments
}

function Get-Sha256 {
  param([string]$Path)

  $stream = [IO.File]::OpenRead($Path)
  $sha256 = [Security.Cryptography.SHA256]::Create()
  try {
    return ([BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '')
  } finally {
    $sha256.Dispose()
    $stream.Dispose()
  }
}

function Assert-PortableLayout {
  param([string]$PortableRoot)

  $expectedTopLevel = @('JobScraper.exe', 'README.txt', 'sidecar')
  $actualTopLevel = @(Get-ChildItem -LiteralPath $PortableRoot | ForEach-Object { $_.Name } | Sort-Object)
  if (($actualTopLevel -join '|') -ne (($expectedTopLevel | Sort-Object) -join '|')) {
    throw "Portable root has unexpected contents: $($actualTopLevel -join ', ')"
  }

  foreach ($required in @(
    'JobScraper.exe',
    'README.txt',
    'sidecar\node.exe',
    'sidecar\worker.mjs',
    'sidecar\adapters.mjs',
    'sidecar\company-adapters.mjs',
    'sidecar\node_modules\playwright-core\package.json',
    'sidecar\node_modules\cheerio\package.json',
    'sidecar\node_modules\fast-xml-parser\package.json',
    'sidecar\node_modules\xpath\package.json',
    'sidecar\node_modules\@xmldom\xmldom\package.json'
  )) {
    if (-not (Test-Path -LiteralPath (Join-Path $PortableRoot $required) -PathType Leaf)) {
      throw "Portable package is missing: $required"
    }
  }

  $sourceFiles = @(Get-ChildItem -LiteralPath $sidecarSource -Recurse -File | ForEach-Object {
    $_.FullName.Substring($sidecarSource.Length + 1)
  } | Sort-Object)
  $portableSidecar = Join-Path $PortableRoot 'sidecar'
  $portableFiles = @(Get-ChildItem -LiteralPath $portableSidecar -Recurse -File | ForEach-Object {
    $_.FullName.Substring($portableSidecar.Length + 1)
  } | Sort-Object)
  if (Compare-Object -ReferenceObject $sourceFiles -DifferenceObject $portableFiles) {
    throw 'Portable sidecar does not exactly match the prepared sidecar file tree.'
  }
}

function Remove-TemporaryDirectory {
  param([string]$Path)

  if (-not (Test-Path -LiteralPath $Path)) { return }
  $fullPath = [IO.Path]::GetFullPath($Path)
  $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
  if (-not $fullPath.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or
      -not ([IO.Path]::GetFileName($fullPath)).StartsWith('JobScraper-', [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to remove unexpected temporary directory: $fullPath"
  }
  Remove-Item -LiteralPath $fullPath -Recurse -Force
}

if ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString() -ne 'X64' -or
    [Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([Runtime.InteropServices.OSPlatform]::Windows) -ne $true) {
  throw 'JobScraper packaging requires Windows x64.'
}

$script:resolvedNode = Resolve-ToolPath -ExplicitPath $NodePath -EnvironmentVariable 'JOBSCRAPER_NODE_RUNTIME' -CommandName 'node'
$script:resolvedPnpm = Resolve-ToolPath -ExplicitPath $PnpmPath -EnvironmentVariable 'JOBSCRAPER_PNPM' -CommandName 'pnpm'
$script:pnpmIsNodeScript = [IO.Path]::GetExtension($script:resolvedPnpm).ToLowerInvariant() -in @('.js', '.cjs', '.mjs')
$cargo = Resolve-ToolPath -CommandName 'cargo'
$rustc = Resolve-ToolPath -CommandName 'rustc'

$nodeVersion = Get-NativeOutput -FilePath $script:resolvedNode -CommandArguments @('--version')
$pnpmVersion = Get-PnpmOutput -CommandArguments @('--version')
$rustInfo = Get-NativeOutput -FilePath $rustc -CommandArguments @('-vV')
if ($nodeVersion -notmatch '^v24\.') { throw "Node 24 is required; found $nodeVersion" }
if ($pnpmVersion -notmatch '^11\.') { throw "pnpm 11 is required; found $pnpmVersion" }
if ($rustInfo -notmatch '(?m)^host:\s+x86_64-pc-windows-msvc\s*$') { throw 'Rust must use the x86_64-pc-windows-msvc host.' }

Push-Location $repo
try {
  Write-Host "Mode: $Mode; Node: $nodeVersion; pnpm: $pnpmVersion"
  Invoke-Pnpm -CommandArguments @('install', '--frozen-lockfile')

  $packageVersion = (Get-Content -LiteralPath (Join-Path $repo 'package.json') -Raw | ConvertFrom-Json).version
  $tauriVersion = (Get-Content -LiteralPath (Join-Path $tauriDir 'tauri.conf.json') -Raw | ConvertFrom-Json).version
  $cargoMetadataJson = Get-NativeOutput -FilePath $cargo -CommandArguments @('metadata', '--no-deps', '--format-version', '1', '--manifest-path', $cargoManifest)
  $cargoMetadata = $cargoMetadataJson | ConvertFrom-Json
  $cargoPackage = @($cargoMetadata.packages | Where-Object { $_.name -eq 'jobscraper' })
  if ($cargoPackage.Count -ne 1) { throw 'Cargo metadata must contain exactly one jobscraper package.' }
  $cargoVersion = $cargoPackage[0].version
  if ($packageVersion -ne $tauriVersion -or $packageVersion -ne $cargoVersion) {
    throw "Version mismatch: package.json=$packageVersion; tauri.conf.json=$tauriVersion; Cargo.toml=$cargoVersion"
  }
  $version = $packageVersion

  if ($Mode -eq 'Dev') {
    & (Join-Path $PSScriptRoot 'prepare-release.ps1') -NodePath $script:resolvedNode -PnpmPath $script:resolvedPnpm
    Invoke-Pnpm -CommandArguments @('tauri', 'dev')
    return
  }

  Write-Host 'Running release gates...'
  Invoke-Pnpm -CommandArguments @('check')
  Invoke-Pnpm -CommandArguments @('test')
  Invoke-Native -FilePath $cargo -CommandArguments @('fmt', '--manifest-path', $cargoManifest, '--', '--check')
  Invoke-Native -FilePath $cargo -CommandArguments @('test', '--manifest-path', $cargoManifest)

  & (Join-Path $PSScriptRoot 'prepare-release.ps1') -NodePath $script:resolvedNode -PnpmPath $script:resolvedPnpm
  Invoke-Pnpm -CommandArguments @('tauri', 'build', '--no-bundle', '--ci')

  $releaseExe = Join-Path $tauriDir 'target\release\jobscraper.exe'
  if (-not (Test-Path -LiteralPath $releaseExe -PathType Leaf)) { throw "Release executable was not created: $releaseExe" }
  $exeVersion = (Get-Item -LiteralPath $releaseExe).VersionInfo.ProductVersion
  if ($exeVersion -ne $version) { throw "Release executable version is $exeVersion; expected $version" }

  $artifactDir = Join-Path $repo "artifacts\$version"
  New-Item -ItemType Directory -Path $artifactDir -Force | Out-Null
  $selectedArtifacts = @()

  if ($Mode -in @('Portable', 'All')) {
    $portableTemp = Join-Path ([IO.Path]::GetTempPath()) "JobScraper-portable-$([guid]::NewGuid().ToString('N'))"
    $verifyTemp = Join-Path ([IO.Path]::GetTempPath()) "JobScraper-verify-$([guid]::NewGuid().ToString('N'))"
    try {
      $portableRoot = Join-Path $portableTemp 'JobScraper'
      New-Item -ItemType Directory -Path $portableRoot -Force | Out-Null
      Copy-Item -LiteralPath $releaseExe -Destination (Join-Path $portableRoot 'JobScraper.exe')
      Copy-Item -LiteralPath $sidecarSource -Destination (Join-Path $portableRoot 'sidecar') -Recurse
      Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'PORTABLE-README.txt') -Destination (Join-Path $portableRoot 'README.txt')
      Assert-PortableLayout -PortableRoot $portableRoot
      Invoke-Native -FilePath $script:resolvedNode -CommandArguments @((Join-Path $PSScriptRoot 'smoke-installed-sidecar.mjs'), $portableRoot)

      $portableArtifact = Join-Path $artifactDir "JobScraper-$version-windows-x64-portable.zip"
      if (Test-Path -LiteralPath $portableArtifact) { Remove-Item -LiteralPath $portableArtifact -Force }
      Compress-Archive -LiteralPath $portableRoot -DestinationPath $portableArtifact -CompressionLevel Optimal

      New-Item -ItemType Directory -Path $verifyTemp -Force | Out-Null
      Expand-Archive -LiteralPath $portableArtifact -DestinationPath $verifyTemp
      $extractedRoot = Join-Path $verifyTemp 'JobScraper'
      Assert-PortableLayout -PortableRoot $extractedRoot
      Invoke-Native -FilePath $script:resolvedNode -CommandArguments @((Join-Path $PSScriptRoot 'smoke-installed-sidecar.mjs'), $extractedRoot)
      $selectedArtifacts += $portableArtifact
    } finally {
      Remove-TemporaryDirectory -Path $portableTemp
      Remove-TemporaryDirectory -Path $verifyTemp
    }
  }

  # Tauri patches the release executable with bundle metadata, so bundle NSIS only
  # after the portable archive has copied the raw --no-bundle executable.
  if ($Mode -in @('Installer', 'All')) {
    Invoke-Pnpm -CommandArguments @('tauri', 'bundle', '--bundles', 'nsis', '--ci')
    $nsisDir = Join-Path $tauriDir 'target\release\bundle\nsis'
    $installers = @(Get-ChildItem -LiteralPath $nsisDir -Filter '*-setup.exe' -File |
      Where-Object { $_.Name -like "*_$($version)_*" })
    if ($installers.Count -ne 1) { throw "Expected one NSIS installer for $version; found $($installers.Count)." }
    $installerArtifact = Join-Path $artifactDir "JobScraper-$version-windows-x64-setup.exe"
    Copy-Item -LiteralPath $installers[0].FullName -Destination $installerArtifact -Force
    $selectedArtifacts += $installerArtifact
  }

  if ($selectedArtifacts.Count -eq 0) { throw "No artifacts were selected for mode $Mode." }
  $hashLines = @($selectedArtifacts | ForEach-Object {
    $hash = Get-Sha256 -Path $_
    "$hash  $([IO.Path]::GetFileName($_))"
  })
  $hashPath = Join-Path $artifactDir 'SHA256SUMS.txt'
  [IO.File]::WriteAllLines($hashPath, $hashLines, [Text.UTF8Encoding]::new($false))

  foreach ($line in [IO.File]::ReadAllLines($hashPath)) {
    if ($line -notmatch '^([A-F0-9]{64})  (.+)$') { throw "Invalid checksum line: $line" }
    $actualHash = Get-Sha256 -Path (Join-Path $artifactDir $Matches[2])
    if ($actualHash -ne $Matches[1]) { throw "Checksum verification failed for $($Matches[2])." }
  }

  Write-Host "Packaging complete: $artifactDir"
} finally {
  Pop-Location
}
