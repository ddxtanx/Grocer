# Grocer — privacy-first grocery receipt & price planning app

## Decisions (settled with owner, 2026-11-06)
- **Client**: PWA. OCR runs **in-browser (WASM)**: Tesseract.wasm baseline, RapidOCR-ONNX-web as the
  accuracy upgrade path. Rust parser crate compiles to the same WASM blob the server ships.
- **Matching**: barcode-assisted. UPCs scanned at the shelf (BarcodeDetector API / zxing-wasm
  fallback) anchor canonical products; receipt lines link via per-store keys (Costco item codes)
  and community-confirmed name→product maps.
- **Price data**: crowdsourced receipts **+ self-hosted scraper** for 3–4 local chains.
  Note: Trader Joe's publishes no online prices — TJ's is receipts-only by nature.
- **Stack**: Axum + Leptos fullstack Rust, one toolchain. Postgres + **SQLx**
  (compile-time-checked queries; the strongly-typed layer Garrett asked for).
- **Deployment**: self-hosted single instance, small trusted group, username/password +
  HttpOnly session cookie.

## Core principle: reliability comes from templates + checksums, not OCR
Raw OCR is treated as *candidate text*. Each chain has a deterministic template parser (Rust
crate `grocer-parser`, runs client-side in WASM) that:
1. Classifies lines (header / department / item / discount / total / payment / junk) and ignores
   everything it doesn't understand.
2. Extracts structure: qty (`2 @ 0.99 EA`), weights (`3.11 lb @ 1.29/lb`), flags (`N F`, `E`, `A`),
   truncated names, repeated lines, coupon lines tied to item codes (`/1961075 5.00-`).
3. **Validates arithmetically** — the receipt must be self-consistent:
   Σ(item lines incl. coupons) == subtotal, subtotal + tax == total, qty × unit ≈ line total,
   Σ qty ≈ item count, tax ≈ rate × taxable lines.
4. **Recovers damaged data** — e.g. a redacted TOTAL is re-derived from subtotal+tax and
   cross-checked against the payment block (the Costco test receipts prove this is needed).
5. Emits per-line confidence + an `issues` list; anything below threshold surfaces a one-tap
   review UI on the device. **Only structured JSON + raw text go to the server — never images.**
6. Strips PII client-side (Costco membership numbers; card PANs are already X-masked).

Dedup: receipts keyed `UNIQUE(store_id, txn_ref, occurred_at)` — the 8 sample photos are 3
transactions; re-scans fold into the same receipt row.

## Corpus (attached receipts)
| Chain | Photo count | Hazards the parser must survive |
|---|---|---|
| Fresh Thyme (St. Louis Park) | 2 (same txn) | dept headers, leading-`0` artifacts, `N F` flags, `You saved` attach to *previous* item, two-line items, weight items, mixed date formats, no tax line |
| Trader Joe's (721 Washington) | 1 | dot-matrix, names truncated ~22 ch, qty as repeated lines + `3 @ $2.49` sub-line, footer-only date, `Tax A amt @ rate amt` |
| Costco (Minneapolis BD #652) | 5 (same txn, some redacted/strikethrough) | item codes, `E`/`A` flags, coupon line `/1961075 5.00-`, redacted TOTAL, double-printed date, membership number (PII) |

## Data model (Postgres, SQLx migrations)
`users, sessions, chains, stores(chain, addr, geo), products(canonical: name, brand, upc,
pack_size, size_unit), trips(user, store, when), receipts(dedup key, totals, raw_text,
parser_version, confidence), receipt_items(raw_name, store_item_code, qty, unit_price,
line_total, flags, savings, →product), price_observations(source: receipt|scraper|manual,
store, product, normalized cents_per_100g), store_item_map(chain, item_code → product,
community confirmations), barcode_links(upc → product), shelf_life_defaults(category days) +
overrides(product), fridge_items(user, product, acquired, expires), shopping_lists +
items, trip_plans + stops + items.`

Cross-store comparison normalizes to **cents per 100 g/ml/each** — mandatory because Costco sells
club packs vs TJ's singles.

## Fridge & shelf life
Receipt item → canonical product → `fridge_item` with `acquired_at = receipt date` and
`expires_at = acquired + shelf_life(category default | product override | user edit)`.
`GET /api/fridge/expiring?days=N` powers "buy X again soon" suggestions that pre-seed lists.

## Shopping list → store plan
Greedy: assign each list item to the store with the freshest best normalized price; evaluate
stop-combos of size 1–2 (and the 3-stop case the corpus implies: FT + TJ's + Costco won't exist
for one list), with haversine distance from home (coords kept client-side; server does the
assignment math on prices only).

## Scraper (phase 3)
Per-chain Rust modules (pluggable trait `ChainCatalog`), run as a cron sidecar on the same host,
polite rate limits, prices tagged with freshness window; receipts always outrank scraped prices
on conflict (observed > advertised). Only chains that publish prices: Target, Aldi, Cub,
Kowalski, Fresh Thyme. UPC resolution via Open Food Facts queried **from the user's browser**
(optionally a self-hosted OFF mirror), keeping external queries on the device per the
architecture rule.

## Milestones
1. **Foundation (this pass)**: parser crate validated against the real corpus (cargo test);
   SQLx server: auth, stores, trips, receipt ingest (server re-verifies checksums), fridge
   + expiring endpoint, price history. Verified against real Postgres.
2. **PWA shell**: Leptos UI + Tesseract.wasm pipeline + review/edit UI + barcode scanning.
3. **Catalog & comparison**: product linking UX, store_item_map voting, price comparison views.
4. **Lists → plans**: planner endpoint + UI; shelf-life seeding; scraper sidecar.
