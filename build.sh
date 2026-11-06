#!/usr/bin/env bash
# Build the whole app: backend binary + in-browser WASM parser, and place the
# wasm where the backend serves it (crates/grocer-server/static/).
#
#   ./build.sh            # release server + release wasm (the default)
#   ./build.sh --debug    # debug server for a fast dev loop
#   ./build.sh --test     # after building: cargo tests + headless wasm/E2E checks
#
# The frontend is static files — no bundler step. The wasm artifact name here
# MUST stay in sync with the fetch() in static/app.js and the PRECACHE list
# in static/sw.js (grocer-parser.wasm).
set -euo pipefail
cd "$(dirname "$0")"

SERVER_PROFILE="release"
RUN_TESTS=0
for arg in "$@"; do
  case "$arg" in
    --debug)   SERVER_PROFILE="debug" ;;
    --test)    RUN_TESTS=1 ;;
    *) echo "usage: $0 [--debug] [--test]" >&2; exit 2 ;;
  esac
done

WASM_TARGET="wasm32-unknown-unknown"
WASM_OUT="target/$WASM_TARGET/release/grocer_wasm.wasm"
WASM_DEST="crates/grocer-server/static/grocer-parser.wasm"

# Fail with an actionable message instead of a cargo wall of text.
# (Non-rustup distro toolchains ship the wasm target in the package; they get
# no preflight and fail at compile time with cargo's own message.)
if command -v rustup >/dev/null 2>&1; then
  if ! rustup target list --installed 2>/dev/null | grep -qx "$WASM_TARGET"; then
    echo "error: rust target '$WASM_TARGET' not installed. Run: rustup target add $WASM_TARGET" >&2
    exit 1
  fi
fi

CARGO_PROFILE=""
[ "$SERVER_PROFILE" = release ] && CARGO_PROFILE="--release"

echo "==> cargo build -p grocer-server ($SERVER_PROFILE)"
cargo build $CARGO_PROFILE -p grocer-server

echo "==> cargo build -p grocer-wasm ($WASM_TARGET, release)"
cargo build -q -p grocer-wasm --target "$WASM_TARGET" --release

echo "==> linking wasm -> $WASM_DEST"
cp "$WASM_OUT" "$WASM_DEST"

# Cheap integrity tripwire: the served file must equal the built one.
if ! cmp -s "$WASM_OUT" "$WASM_DEST"; then
  echo "error: served wasm differs from built wasm" >&2
  exit 1
fi

BIN="target/$SERVER_PROFILE/grocer-server"
echo
echo "built:"
ls -la "$BIN" "$WASM_DEST"
echo
if [ ! -f .env ]; then
  echo "note: no .env in repo root — copy .env.example to .env (DATABASE_URL, HOST, PORT)"
fi
echo "run:  $BIN"

if [ "$RUN_TESTS" = 1 ]; then
  echo "==> cargo test --workspace"
  cargo test -q --workspace
  echo "==> node tools/wasm_smoke.js"
  node tools/wasm_smoke.js
  if curl -sf -o /dev/null http://127.0.0.1:8787/ 2>/dev/null; then
    echo "==> node tools/e2e_wasm_client.js (server reachable)"
    node tools/e2e_wasm_client.js
  else
    echo "==> skipping E2E (no server on :8787)"
  fi
fi
