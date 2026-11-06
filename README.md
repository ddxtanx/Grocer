# Grocer

Privacy-first grocery receipt & price planning. Self-hosted; small trusted group.

## How it works

1. **Scan a receipt photo in the PWA.** OCR runs entirely in your browser
   (vendored Tesseract.wasm — no CDN, no image upload).
2. **The receipt is parsed in your browser** by the `grocer-parser` Rust crate
   compiled to WASM. Chain templates + arithmetic checksums (Σlines=subtotal,
   subtotal+tax=total, qty×unit=line, Σqty=item count) certify the data before
   it is sent. You review/edit, then save. Only structured JSON + OCR text
   reach the server — never the photo.
3. **Product matching** is barcode-anchored and community-built: a shelf UPC
   scan pins a product; chain item codes (Costco) map via voted
   `store_item_map`, so later receipts auto-link for everyone in the group.
4. **Shelf-tag checks:** photograph an advertised price tag; when the trip's
   receipt lands, `/api/trips/{id}/price-integrity` verdicts every tag as
   MATCH / OVERCHARGE / UNDERCHARGE.
5. **Fridge:** receipt items enter the fridge with category shelf-life
   defaults; expiring items seed restock lists.
6. **Lists → plans:** the planner assigns each item to the store with the
   best known price. Receipts outrank scraper prices (recent receipts win
   within 30 days); the `grocer-scraper` sidecar fills catalog gaps.

## Crates

| Crate | What |
|---|---|
| `grocer-parser` | chain templates (Costco, Trader Joe's, Fresh Thyme), checksum validation, PII masking. Test corpus = your real receipts. |
| `grocer-wasm` | same parser as a browser WASM module, plain C ABI (`alloc/parse/free_str`), no wasm-bindgen. |
| `grocer-server` | Axum API + static PWA, SQLx/Postgres, argon2 auth + hashed session cookies, dedup, auto-linking, fridge, lists, plans, price integrity. |
| `grocer-scraper` | cron sidecar; `ChainCatalog` trait per chain; pure-tested HTML parsers; receipts-outrank-scrapers rule. |

## Build & run

```sh
# native tests
cargo test --workspace

# WASM parser artifact (needs the wasm32-unknown-unknown target)
cargo build -p grocer-wasm --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/grocer_wasm.wasm crates/grocer-server/static/grocer-parser.wasm

# database (first time)
initdb -D .pgdata -U postgres --auth=trust
pg_ctl -D .pgdata -o "-p 5435 -k /tmp -c listen_addresses=localhost" start
psql -h /tmp -p 5435 -U postgres -c "CREATE ROLE grocer LOGIN PASSWORD 'grocer';"
psql -h /tmp -p 5435 -U postgres -c "CREATE DATABASE grocer OWNER grocer;"

# server (migrations run automatically, opens http://127.0.0.1:8787)
DATABASE_URL=postgres://grocer:grocer@localhost:5435/grocer ./target/debug/grocer-server

# headless end-to-end checks (needs server + DB running)
node tools/wasm_smoke.js        # WASM artifact vs the real receipt corpus
node tools/e2e_wasm_client.js   # WASM parse -> ingest -> totals for all chains

# scraper sidecar
./target/debug/grocer-scraper fixture http://localhost:8899/fixture_catalog.html
```

## Frontend status

Currently hand-written JS (`static/app.js`) against the JSON API — every
flow is real and exercised by the E2E tools. The planned Leptos/Trunk
upgrade is a UI-surface change only: the API, the WASM parser ABI, and the
data model stay identical. Trunk is not in the sandbox image yet.

## Out of scope (by design)

No image storage, no external APIs called from the server, no cloud OCR.
UPC enrichment (Open Food Facts) happens from the user's browser only.
