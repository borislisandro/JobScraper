# Packaged offline BGE model cache

Populate this directory at release build time with `scripts/fetch-bge-model.ps1` and include it as a Tauri resource. The app checks for `Xenova/bge-small-en-v1.5/onnx/model.onnx` before it initializes FastEmbed and sets `HF_HUB_OFFLINE=1`; it will fail locally rather than download a model.
