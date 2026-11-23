#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR/web"

if [[ "${1:-}" == "--rebuild" || ! -f "src/wasm/sistars_engine_bg.wasm" ]]; then
  echo "Building Rust WASM engine..."
  npm run build:wasm
fi

HOST_IP="$(hostname -I | awk '{print $1}')"
PROTO="http"
if [[ -f "$ROOT_DIR/web/.certs/key.pem" && -f "$ROOT_DIR/web/.certs/cert.pem" ]]; then
  PROTO="https"
fi
echo "Starting SI-Stars Vite dev server on ${PROTO}://${HOST_IP:-0.0.0.0}:5173 ..."
exec npm run dev -- --host 0.0.0.0
