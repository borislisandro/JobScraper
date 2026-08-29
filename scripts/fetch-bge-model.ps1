param([string]$Destination = (Join-Path $PSScriptRoot '..\src-tauri\resources\models\bge-small-en-v1.5'))
$ErrorActionPreference = 'Stop'
$revision = 'ea104dacec62c0de699686887e3f920caeb4f3e3'
$repo = 'Xenova/bge-small-en-v1.5'
$files = @(
  @{ Source = 'config.json'; Target = 'config.json' },
  @{ Source = 'tokenizer.json'; Target = 'tokenizer.json' },
  @{ Source = 'tokenizer_config.json'; Target = 'tokenizer_config.json' },
  @{ Source = 'special_tokens_map.json'; Target = 'special_tokens_map.json' },
  @{ Source = 'onnx/model.onnx'; Target = 'model.onnx' }
)
$target = [IO.Path]::GetFullPath($Destination)
New-Item -ItemType Directory -Force -Path $target | Out-Null
foreach ($file in $files) {
  Invoke-WebRequest -Uri "https://huggingface.co/$repo/resolve/$revision/$($file.Source)" -OutFile (Join-Path $target $file.Target)
}
Write-Host "Fetched pinned BAAI/bge-small-en-v1.5-compatible ONNX files at $target. Run prepare-release.ps1 -PreflightOnly to validate hashes."
