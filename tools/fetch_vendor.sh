#!/usr/bin/env bash
# Re-download the pinned OCR vendor assets and verify against vendor.lock.
# The CDN is touched HERE (build time, by the operator) — never at runtime:
# the browser only ever talks to the grocer server (privacy + offline).
#
# To upgrade tesseract: bump the versions below, run this script, then
# regenerate vendor.lock with:  (cd static/vendor && sha256sum <files> > ../../../vendor.lock)
# and update the pins in this file. Then re-run the OCR browser test.
set -euo pipefail
cd "$(dirname "$0")/.."

TESSERACT_JS=5.1.1            # main lib (pinned: worker protocol must match)
CORE=5.1.0                    # wasm core files (v5 naming: tesseract-core{,-simd}.wasm*)
                              # NOTE: tesseract.js 5.1.1 defaults to core-v6 '-lstm'
                              # filenames; app.js pins corePath to the exact v5 glue
                              # filename so this pairing is deterministic.

D=crates/grocer-server/static/vendor
mkdir -p "$D/tessdata"

fetch() { curl -fsSL "$1" -o "$2"; }
fetch "https://unpkg.com/tesseract.js@$TESSERACT_JS/dist/tesseract.min.js"        "$D/tesseract.min.js"
fetch "https://unpkg.com/tesseract.js@$TESSERACT_JS/dist/worker.min.js"           "$D/worker.min.js"
fetch "https://unpkg.com/tesseract.js-core@$CORE/tesseract-core-simd.wasm.js"     "$D/tesseract-core-simd.wasm.js"
fetch "https://unpkg.com/tesseract.js-core@$CORE/tesseract-core-simd.wasm"        "$D/tesseract-core-simd.wasm"
fetch "https://unpkg.com/tesseract.js-core@$CORE/tesseract-core.wasm.js"          "$D/tesseract-core.wasm.js"
fetch "https://unpkg.com/tesseract.js-core@$CORE/tesseract-core.wasm"             "$D/tesseract-core.wasm"
curl -fsSL "https://tessdata.projectnaptha.com/4.0.0/eng.traineddata.gz" -o "$D/tessdata/eng.traineddata.gz"

echo "==> verifying vendor.lock"
sha256sum -c "$OLDPWD/vendor.lock" --directory "$D" 2>/dev/null || (cd "$D" && sha256sum -c "$OLDPWD/vendor.lock")
echo "vendor OK"
