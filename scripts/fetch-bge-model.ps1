param([Parameter(Mandatory=$true)][string]$Destination)
$ErrorActionPreference='Stop'
# Release/build-time action only. The application never calls this and runs with HF_HUB_OFFLINE=1.
$root=Join-Path $Destination 'Xenova\bge-small-en-v1.5'
New-Item -ItemType Directory -Force -Path (Join-Path $root 'onnx') | Out-Null
$files=@('config.json','tokenizer.json','tokenizer_config.json','special_tokens_map.json')
foreach($file in $files){Invoke-WebRequest -Uri "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/$file" -OutFile (Join-Path $root $file)}
Invoke-WebRequest -Uri 'https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/onnx/model.onnx' -OutFile (Join-Path $root 'onnx\model.onnx')
Get-FileHash (Join-Path $root 'onnx\model.onnx') -Algorithm SHA256
