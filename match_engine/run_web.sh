#!/usr/bin/env bash
# Build the WASM engine and serve the Canvas front end.
#
#   ./run_web.sh            # build + serve on :8000
#   ./run_web.sh 9000       # build + serve on :9000
#
# Then open http://localhost:<port>/web/
set -euo pipefail
cd "$(dirname "$0")"

PORT="${1:-8000}"

# ── Toolchain ──────────────────────────────────────────────────
if ! rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
  echo "› adding wasm32 target…"
  rustup target add wasm32-unknown-unknown
fi
if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "› installing wasm-bindgen-cli…"
  cargo install wasm-bindgen-cli
fi

# ── Build ──────────────────────────────────────────────────────
echo "› compiling to wasm32…"
cargo build --target wasm32-unknown-unknown --release
echo "› generating JS bindings (web/pkg/)…"
wasm-bindgen --target web --out-dir web/pkg \
  target/wasm32-unknown-unknown/release/match_engine.wasm

# ── Seed the bundled fixtures + match manifest ────────
echo "› copying bundled fixtures into web/matches/"
mkdir -p web/matches
cp data/*.xml web/matches/
echo "› generating web/matches.json…"
cargo run --release --example gen_manifest

# ── Serve ──────────────────────────────────────────────────────
PY=python
command -v python >/dev/null 2>&1 || PY=python3
echo ""
echo "════════════════════════════════════════════════════════"
echo "  Open  http://localhost:${PORT}/web/"
echo "  Stop  Ctrl+C"
echo "════════════════════════════════════════════════════════"
echo ""
exec "$PY" -m http.server "$PORT"
