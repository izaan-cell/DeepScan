#!/bin/bash
# Downloads the ONNX models + tokenizers DeepScan needs and places them
# under $1 (created if missing). Shared between macOS and Windows CI (the
# Windows release job already runs its steps under `shell: bash`).
#
# Model choices verified directly against their actual ONNX graphs (input/
# output tensor names, dimensions) — see rust-engine/src/models.rs and the
# commit that introduced this script for what was checked and why:
#   - CLIP vision (quantized): pixel_values -> image_embeds[512]
#   - MiniLM (quantized): input_ids+attention_mask+token_type_ids -> 384-d
#   - Jina-Code (quantized): input_ids+attention_mask (no token_type_ids) -> 768-d
#
# Quantized variants are used deliberately to keep the installer size sane
# (~275MB total here vs. ~1.3GB for the fp32 originals).
set -euo pipefail

DEST="${1:?usage: download-models.sh <destination-dir>}"
mkdir -p "$DEST"

fetch() {
  local url="$1" out="$DEST/$2"
  if [ -f "$out" ]; then
    echo "==> $2 already present, skipping"
    return
  fi
  echo "==> Downloading $2"
  curl -sL --fail -o "$out.partial" "$url"
  mv "$out.partial" "$out"
}

fetch "https://huggingface.co/Xenova/clip-vit-base-patch32/resolve/main/onnx/vision_model_quantized.onnx" \
  "clip-vit-b32.onnx"

fetch "https://huggingface.co/Xenova/all-MiniLM-L6-v2/resolve/main/onnx/model_quantized.onnx" \
  "all-MiniLM-L6-v2.onnx"
fetch "https://huggingface.co/Xenova/all-MiniLM-L6-v2/resolve/main/tokenizer.json" \
  "all-MiniLM-L6-v2.tokenizer.json"

fetch "https://huggingface.co/jinaai/jina-embeddings-v2-base-code/resolve/main/onnx/model_quantized.onnx" \
  "jina-embeddings-v2-base-code.onnx"
fetch "https://huggingface.co/jinaai/jina-embeddings-v2-base-code/resolve/main/tokenizer.json" \
  "jina-code.tokenizer.json"

echo "==> Models ready in $DEST"
du -sh "$DEST"
