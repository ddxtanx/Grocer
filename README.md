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
# one command: backend binary + browser WASM, wasm auto-linked into static/
./build.sh              # release server + release wasm
./build.sh --debug      # debug server for a fast dev loop
./build.sh --test       # then: cargo tests + headless wasm smoke + E2E

# database (first time)
initdb -D .pgdata -U postgres --auth=trust
pg_ctl -D .pgdata -o "-p 5435 -k /tmp -c listen_addresses=localhost" start
psql -h /tmp -p 5435 -U postgres -c "CREATE ROLE grocer LOGIN PASSWORD 'grocer';"
psql -h /tmp -p 5435 -U postgres -c "CREATE DATABASE grocer OWNER grocer;"

# configuration: copy the example, edit to taste; both binaries load .env
# from the repo root (real env vars win). HOST controls the bind address —
# default 127.0.0.1; set 0.0.0.0 or a LAN address to serve other devices.
cp .env.example .env

# server (migrations run automatically; binds HOST:PORT from .env)
./target/release/grocer-server

# headless end-to-end checks (needs server + DB running; also run by --test)
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

## Scraper reality check (2026-11-06 probe)

Server-side `ureq` scraping of the local chains does not work: **shopcub.com is a
parked domain** (Cub's storefront is cub.com, an Albertsons-style JS SPA),
**Fresh Thyme** is a mi9cloud JS microsite (`/sm/` paths), and Trader Joe's
publishes no online prices at all. These sites sit behind JS rendering and
aggressive bot walls; a polite sidecar gets nothing parseable.

Revised plan (matches the project's core pattern — heavy work moves to the
device): chain catalog fetch becomes a **browser-side module** in the PWA,
fetching from the user's own browser session (same principle as Open Food
Facts lookups staying on-device). Parser modules are pure functions over the
fetched DOM, unit-tested against saved fixtures, and results POST to
`/api/price-checks`/observations with `source='scraper'` — the receipts-outrank
rule is already live and enforced server-side (proven end-to-end). The
`grocer-scraper` sidecar remains for any future chain that *does* serve static
HTML (weekly-ad sites, APIs that opt in).

## Out of scope (by design)

No image storage, no external APIs called from the server, no cloud OCR.
UPC enrichment (Open Food Facts) happens from the user's browser only.
