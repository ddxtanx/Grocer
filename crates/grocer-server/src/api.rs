use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::NaiveDate;
use grocer_parser::{Chain, IssueKind, ParsedReceipt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;

use crate::auth::Authed;
use crate::error::{ApiResult, AppError};
use crate::AppState;

fn chain_parser_key(c: Chain) -> Option<&'static str> {
    match c {
        Chain::FreshThyme => Some("fresh_thyme"),
        Chain::TraderJoes => Some("trader_joes"),
        Chain::Costco => Some("costco"),
        Chain::Unknown => None,
    }
}

// ---------- parse_text (provisional home of the parser) ----------
//
// Until the sandbox has the wasm32 target (rust-wasm package) and the parser
// runs in-browser via WASM, the client posts OCR text here and gets back the
// structured ParsedReceipt. Same crate, same code — only the execution
// location differs, and the client flips to WASM by deleting this round-trip.

#[derive(Deserialize)]
pub struct ParseTextReq {
    pub raw_text: String,
}

pub async fn parse_text(
    State(_st): State<AppState>,
    _authed: Authed,
    Json(p): Json<ParseTextReq>,
) -> ApiResult<Json<ParsedReceipt>> {
    Ok(Json(grocer_parser::parse(&p.raw_text)))
}

// ---------- product search & unlinked items (catalog curation UI) ----------

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

pub async fn search_products(
    State(st): State<AppState>,
    _authed: Authed,
    Query(s): Query<SearchQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let rows =
        match &s.q {
            Some(q) if !q.is_empty() => sqlx::query(
                "SELECT id, name, brand, category, upc, pack_size::float8 AS pack_size, size_unit
                 FROM products
                 WHERE name ILIKE $1 OR brand ILIKE $1 ORDER BY name LIMIT 50",
            )
            .bind(format!("%{q}%"))
            .fetch_all(&st.pool)
            .await?,
            _ => sqlx::query(
                "SELECT id, name, brand, category, upc, pack_size::float8 AS pack_size, size_unit
                 FROM products ORDER BY id DESC LIMIT 50",
            )
            .fetch_all(&st.pool)
            .await?,
        };
    let out: Vec<_> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<i64,_>("id"),
                "name": r.get::<String,_>("name"),
                "brand": r.get::<Option<String>,_>("brand"),
                "category": r.get::<Option<String>,_>("category"),
                "upc": r.get::<Option<String>,_>("upc"),
                "pack_size": r.get::<Option<f64>,_>("pack_size"),
                "size_unit": r.get::<Option<String>,_>("size_unit"),
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

/// Receipt lines not yet tied to a canonical product — the work queue for
/// catalog curation, with existing mapping candidates attached.
pub async fn unlinked_items(
    State(st): State<AppState>,
    _authed: Authed,
) -> ApiResult<Json<serde_json::Value>> {
    let rows = sqlx::query(
        "SELECT ri.id, ri.raw_name, ri.store_item_code, ri.unit_price_cents,
                c.name AS chain, s.name AS store, r.occurred_at,
                COALESCE(
                    (SELECT json_agg(json_build_object('product_id', m.product_id,
                        'confirmations', m.confirmations))
                     FROM store_item_map m
                     WHERE m.chain_id = r.chain_id AND m.store_item_code = ri.store_item_code),
                    '[]'::json) AS candidates
         FROM receipt_items ri
         JOIN receipts r ON r.id = ri.receipt_id
         JOIN stores s ON s.id = r.store_id
         JOIN chains c ON c.id = r.chain_id
         WHERE ri.product_id IS NULL AND ri.discount_ref IS NULL
           AND ri.unit_price_cents IS NOT NULL
         ORDER BY r.occurred_at DESC NULLS LAST LIMIT 100",
    )
    .fetch_all(&st.pool)
    .await?;
    let out: Vec<_> = rows
        .iter()
        .map(|r| {
            json!({
                "item_id": r.get::<i64,_>("id"),
                "raw_name": r.get::<String,_>("raw_name"),
                "store_item_code": r.get::<Option<String>,_>("store_item_code"),
                "unit_price_cents": r.get::<Option<i64>,_>("unit_price_cents"),
                "chain": r.get::<String,_>("chain"),
                "store": r.get::<String,_>("store"),
                "occurred_at": r.get::<Option<String>,_>("occurred_at"),
                "candidates": r.get::<serde_json::Value,_>("candidates"),
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

// ---------- stores ----------

pub async fn list_stores(
    State(st): State<AppState>,
    _authed: Authed,
) -> ApiResult<Json<serde_json::Value>> {
    let rows = sqlx::query(
        "SELECT s.id, s.name, s.address, s.city, s.store_ref, c.name AS chain \
         FROM stores s JOIN chains c ON c.id = s.chain_id ORDER BY c.name, s.name",
    )
    .fetch_all(&st.pool)
    .await?;
    let stores: Vec<_> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<i64, _>("id"),
                "chain": r.get::<String, _>("chain"),
                "name": r.get::<String, _>("name"),
                "address": r.get::<Option<String>, _>("address"),
                "city": r.get::<Option<String>, _>("city"),
                "store_ref": r.get::<Option<String>, _>("store_ref"),
            })
        })
        .collect();
    Ok(Json(json!(stores)))
}

// ---------- trips ----------

#[derive(Deserialize)]
pub struct TripReq {
    pub store_id: i64,
    pub occurred_at: String,
}

pub async fn create_trip(
    State(st): State<AppState>,
    authed: Authed,
    Json(t): Json<TripReq>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let row = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO trips (user_id, store_id, occurred_at) VALUES ($1,$2,$3) RETURNING id",
    )
    .bind(authed.user_id)
    .bind(t.store_id)
    .bind(&t.occurred_at)
    .fetch_one(&st.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": row.0 }))))
}

pub async fn my_trips(
    State(st): State<AppState>,
    authed: Authed,
) -> ApiResult<Json<serde_json::Value>> {
    let rows = sqlx::query(
        "SELECT t.id, t.occurred_at, s.name AS store, s.id AS store_id
         FROM trips t JOIN stores s ON s.id = t.store_id
         WHERE t.user_id=$1 ORDER BY t.occurred_at DESC LIMIT 50",
    )
    .bind(authed.user_id)
    .fetch_all(&st.pool)
    .await?;
    let out: Vec<_> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<i64,_>("id"),
                "occurred_at": r.get::<String,_>("occurred_at"),
                "store": r.get::<String,_>("store"),
                "store_id": r.get::<i64,_>("store_id"),
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
pub struct IngestReq {
    pub trip_id: Option<i64>,
    /// explicit store override; otherwise resolved from chain + store_ref
    pub store_id: Option<i64>,
    pub receipt: ParsedReceipt,
}

#[derive(Serialize)]
pub struct IngestResp {
    receipt_id: i64,
    deduplicated: bool,
    issues: Vec<String>,
}

pub async fn ingest_receipt(
    State(st): State<AppState>,
    authed: Authed,
    Json(req): Json<IngestReq>,
) -> ApiResult<(StatusCode, Json<IngestResp>)> {
    // Server-side re-verification: never trust a client's arithmetic.
    let issues = grocer_parser::verify(&req.receipt);
    let hard: Vec<_> = issues
        .iter()
        .filter(|i| {
            matches!(
                i.kind,
                IssueKind::SumMismatch | IssueKind::LineMathMismatch | IssueKind::TaxMismatch
            )
        })
        .collect();
    if !hard.is_empty() {
        let details: Vec<String> = hard.iter().map(|i| i.detail.clone()).collect();
        return Err(AppError::Unprocessable(format!(
            "receipt failed arithmetic verification: {details:?}"
        )));
    }

    let key = chain_parser_key(req.receipt.chain)
        .ok_or_else(|| AppError::BadRequest("unknown chain".into()))?;
    let store_id = match req.store_id {
        Some(id) => id,
        None => {
            let store_ref = req
                .receipt
                .store_ref
                .clone()
                .ok_or_else(|| AppError::BadRequest("no store_ref to resolve".into()))?;
            sqlx::query_as::<_, (i64,)>(
                "SELECT s.id FROM stores s JOIN chains c ON c.id=s.chain_id \
                 WHERE c.parser_key=$1 AND s.store_ref=$2",
            )
            .bind(key)
            .bind(&store_ref)
            .fetch_optional(&st.pool)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "unknown store {key}/{store_ref} — register it first"
                ))
            })?
            .0
        }
    };
    let chain_id: (i64,) =
        sqlx::query_as("SELECT c.id FROM chains c JOIN stores s ON s.chain_id=c.id WHERE s.id=$1")
            .bind(store_id)
            .fetch_one(&st.pool)
            .await?;

    // Dedup: same store + txn_ref + timestamp = same receipt (re-scans fold in).
    let existing = sqlx::query(
        "SELECT id FROM receipts WHERE store_id=$1 \
         AND txn_ref IS NOT DISTINCT FROM $2 AND occurred_at IS NOT DISTINCT FROM $3",
    )
    .bind(store_id)
    .bind(&req.receipt.txn_ref)
    .bind(&req.receipt.occurred_at)
    .fetch_optional(&st.pool)
    .await?;
    if let Some(row) = existing {
        return Ok((
            StatusCode::OK,
            Json(IngestResp {
                receipt_id: row.get("id"),
                deduplicated: true,
                issues: vec![],
            }),
        ));
    }

    let mut tx = st.pool.begin().await?;
    let rec = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO receipts (user_id, trip_id, store_id, chain_id, txn_ref, occurred_at,
            subtotal_cents, tax_cents, total_cents, discount_cents, item_count,
            total_source, raw_text, parser_version, issues)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15) RETURNING id",
    )
    .bind(authed.user_id)
    .bind(req.trip_id)
    .bind(store_id)
    .bind(chain_id.0)
    .bind(&req.receipt.txn_ref)
    .bind(&req.receipt.occurred_at)
    .bind(req.receipt.subtotal_cents)
    .bind(req.receipt.tax_cents)
    .bind(req.receipt.total_cents)
    .bind(req.receipt.discount_total_cents)
    .bind(req.receipt.item_count)
    .bind(format!("{:?}", req.receipt.total_source))
    .bind(&req.receipt.raw_text)
    .bind(&req.receipt.parser_version)
    .bind(serde_json::to_value(&req.receipt.issues).unwrap())
    .fetch_one(&mut *tx)
    .await?;

    for l in &req.receipt.lines {
        // Auto-link: if the community already mapped this chain's item code
        // (or the user anchored it via a shelf barcode), the line links itself.
        let mut product_id: Option<i64> = None;
        if let Some(code) = &l.store_item_code {
            product_id = sqlx::query_as::<_, (i64,)>(
                "SELECT product_id FROM store_item_map
                 WHERE chain_id=$1 AND store_item_code=$2
                 ORDER BY confirmations DESC LIMIT 1",
            )
            .bind(chain_id.0)
            .bind(code)
            .fetch_optional(&mut *tx)
            .await?
            .map(|r| r.0);
        }

        let item = sqlx::query_as::<_, (i64,)>(
            "INSERT INTO receipt_items (receipt_id, seq, raw_name, store_item_code, discount_ref,
                qty_count, qty_grams, unit_basis, unit_price_cents, line_total_cents,
                savings_cents, flags, department, confidence, product_id)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15) RETURNING id",
        )
        .bind(rec.0)
        .bind(l.seq as i32)
        .bind(&l.raw_name)
        .bind(&l.store_item_code)
        .bind(&l.discount_ref)
        .bind(l.qty_count)
        .bind(l.qty_grams)
        .bind(&l.unit_basis)
        .bind(l.unit_price_cents)
        .bind(l.line_total_cents)
        .bind(l.savings_cents)
        .bind(serde_json::to_value(&l.flags).unwrap())
        .bind(&l.department)
        .bind(l.confidence)
        .bind(product_id)
        .fetch_one(&mut *tx)
        .await?;

        // Price observations: real purchased prices (skip coupons & flag-only lines).
        if l.discount_ref.is_none() {
            if let Some(unit) = l.unit_price_cents {
                sqlx::query(
                    "INSERT INTO price_observations (store_id, chain_id, receipt_item_id,
                        product_id, store_item_code, raw_name, qty_count, qty_grams,
                        unit_price_cents, observed_at)
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
                )
                .bind(store_id)
                .bind(chain_id.0)
                .bind(item.0)
                .bind(product_id)
                .bind(&l.store_item_code)
                .bind(&l.raw_name)
                .bind(l.qty_count)
                .bind(l.qty_grams)
                .bind(unit)
                .bind(req.receipt.occurred_at.clone().unwrap_or_default())
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    tx.commit().await?;

    let soft: Vec<String> = issues
        .iter()
        .map(|i| format!("{:?}: {}", i.kind, i.detail))
        .collect();
    Ok((
        StatusCode::CREATED,
        Json(IngestResp {
            receipt_id: rec.0,
            deduplicated: false,
            issues: soft,
        }),
    ))
}

pub async fn my_receipts(
    State(st): State<AppState>,
    authed: Authed,
) -> ApiResult<Json<serde_json::Value>> {
    let rows = sqlx::query(
        "SELECT r.id, s.name AS store, c.name AS chain, r.occurred_at, r.total_cents, r.txn_ref
         FROM receipts r JOIN stores s ON s.id=r.store_id JOIN chains c ON c.id=r.chain_id
         WHERE r.user_id=$1 ORDER BY r.occurred_at DESC",
    )
    .bind(authed.user_id)
    .fetch_all(&st.pool)
    .await?;
    let out: Vec<_> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<i64,_>("id"),
                "store": r.get::<String,_>("store"),
                "chain": r.get::<String,_>("chain"),
                "occurred_at": r.get::<Option<String>,_>("occurred_at"),
                "total_cents": r.get::<Option<i64>,_>("total_cents"),
                "txn_ref": r.get::<Option<String>,_>("txn_ref"),
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

pub async fn receipt_detail(
    State(st): State<AppState>,
    authed: Authed,
    Path(id): Path<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    let rec = sqlx::query("SELECT * FROM receipts WHERE id=$1 AND user_id=$2")
        .bind(id)
        .bind(authed.user_id)
        .fetch_optional(&st.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("receipt not found".into()))?;
    let items = sqlx::query(
        "SELECT id, seq, raw_name, store_item_code, qty_count, qty_grams::float8 AS qty_grams,
                unit_price_cents, line_total_cents, savings_cents, product_id FROM receipt_items
         WHERE receipt_id=$1 ORDER BY seq",
    )
    .bind(id)
    .fetch_all(&st.pool)
    .await?;
    let out: Vec<_> = items
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<i64, _>("id"),
                "seq": r.get::<i32, _>("seq"),
                "name": r.get::<String,_>("raw_name"),
                "store_item_code": r.get::<Option<String>,_>("store_item_code"),
                "qty": r.get::<Option<i32>,_>("qty_count"),
                "grams": r.get::<Option<f64>,_>("qty_grams"),
                "unit_price_cents": r.get::<Option<i64>,_>("unit_price_cents"),
                "line_total_cents": r.get::<i64,_>("line_total_cents"),
                "savings_cents": r.get::<Option<i64>,_>("savings_cents"),
                "product_id": r.get::<Option<i64>,_>("product_id"),
            })
        })
        .collect();
    Ok(Json(json!({
        "id": rec.get::<i64,_>("id"),
        "total_cents": rec.get::<Option<i64>,_>("total_cents"),
        "total_source": rec.get::<Option<String>,_>("total_source"),
        "items": out,
    })))
}

// ---------- products, barcode & item-code linking ----------

#[derive(Deserialize)]
pub struct ProductReq {
    pub name: String,
    pub brand: Option<String>,
    pub category: Option<String>,
    pub upc: Option<String>,
    pub pack_size: Option<f64>,
    pub size_unit: Option<String>,
}

pub async fn create_product(
    State(st): State<AppState>,
    _authed: Authed,
    Json(p): Json<ProductReq>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let row = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO products (name, brand, category, upc, pack_size, size_unit)
         VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
    )
    .bind(&p.name)
    .bind(&p.brand)
    .bind(&p.category)
    .bind(&p.upc)
    .bind(p.pack_size)
    .bind(&p.size_unit)
    .fetch_one(&st.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": row.0 }))))
}

#[derive(Deserialize)]
pub struct LinkReq {
    pub product_id: i64,
}

/// Link one receipt line to a canonical product. If the line carries a chain item
/// code (Costco), the (chain, code) -> product mapping is remembered and voted on —
/// that's what makes later receipts auto-link.
pub async fn link_receipt_item(
    State(st): State<AppState>,
    _authed: Authed,
    Path(item_id): Path<i64>,
    Json(l): Json<LinkReq>,
) -> ApiResult<Json<serde_json::Value>> {
    let item = sqlx::query(
        "SELECT ri.store_item_code, r.chain_id FROM receipt_items ri
         JOIN receipts r ON r.id = ri.receipt_id WHERE ri.id=$1",
    )
    .bind(item_id)
    .fetch_optional(&st.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("item not found".into()))?;
    let chain_id: i64 = item.get("chain_id");
    let code: Option<String> = item.get("store_item_code");

    let mut tx = st.pool.begin().await?;
    sqlx::query("UPDATE receipt_items SET product_id=$1 WHERE id=$2")
        .bind(l.product_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE price_observations SET product_id=$1 WHERE receipt_item_id=$2")
        .bind(l.product_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    if let Some(code) = &code {
        sqlx::query(
            "INSERT INTO store_item_map (chain_id, store_item_code, product_id)
             VALUES ($1,$2,$3)
             ON CONFLICT (chain_id, store_item_code, product_id)
             DO UPDATE SET confirmations = store_item_map.confirmations + 1",
        )
        .bind(chain_id)
        .bind(code)
        .bind(l.product_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct BarcodeReq {
    pub upc: String,
    pub product_id: i64,
}

pub async fn link_barcode(
    State(st): State<AppState>,
    _authed: Authed,
    Json(b): Json<BarcodeReq>,
) -> ApiResult<Json<serde_json::Value>> {
    sqlx::query(
        "INSERT INTO barcode_links (upc, product_id) VALUES ($1,$2)
         ON CONFLICT (upc) DO UPDATE SET product_id = EXCLUDED.product_id,
             confirmations = barcode_links.confirmations + 1",
    )
    .bind(&b.upc)
    .bind(b.product_id)
    .execute(&st.pool)
    .await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct PriceQuery {
    pub product_id: Option<i64>,
    pub store_item_code: Option<String>,
    pub upc: Option<String>,
}

/// Cross-store price history for one product — the comparison core.
pub async fn price_history(
    State(st): State<AppState>,
    _authed: Authed,
    Query(q): Query<PriceQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let product_id = match q.product_id {
        Some(p) => Some(p),
        None => match &q.upc {
            Some(upc) => {
                sqlx::query_as::<_, (i64,)>("SELECT product_id FROM barcode_links WHERE upc=$1")
                    .bind(upc)
                    .fetch_optional(&st.pool)
                    .await
                    .map(|r| r.map(|x| x.0))?
            }
            None => None,
        },
    };
    let rows = if let Some(pid) = product_id {
        sqlx::query(
            "SELECT o.id, o.unit_price_cents, o.qty_count, o.qty_grams::float8 AS qty_grams,
                    o.observed_at, o.source, s.name AS store, c.name AS chain,
                    p.pack_size::float8 AS pack_size, p.size_unit
             FROM price_observations o
             JOIN stores s ON s.id=o.store_id JOIN chains c ON c.id=o.chain_id
             LEFT JOIN products p ON p.id=o.product_id
             WHERE o.product_id=$1 ORDER BY o.observed_at DESC LIMIT 200",
        )
        .bind(pid)
        .fetch_all(&st.pool)
        .await?
    } else if let Some(code) = &q.store_item_code {
        sqlx::query(
            "SELECT o.id, o.unit_price_cents, o.qty_count, o.qty_grams::float8 AS qty_grams,
                    o.observed_at, o.source, s.name AS store, c.name AS chain,
                    p.pack_size::float8 AS pack_size, p.size_unit
             FROM price_observations o
             JOIN stores s ON s.id=o.store_id JOIN chains c ON c.id=o.chain_id
             LEFT JOIN products p ON p.id=o.product_id
             WHERE o.store_item_code=$1 ORDER BY o.observed_at DESC LIMIT 200",
        )
        .bind(code)
        .fetch_all(&st.pool)
        .await?
    } else {
        return Err(AppError::BadRequest(
            "product_id, upc or store_item_code required".into(),
        ));
    };
    let out: Vec<_> = rows
        .iter()
        .map(|r| {
            // Normalized comparison basis: cents per 100 g/ml. Prefer the
            // observation's own weight (receipt weight items), else the
            // canonical product's pack size.
            let unit = r.get::<i64, _>("unit_price_cents");
            let grams = r
                .get::<Option<f64>, _>("qty_grams")
                .filter(|g| *g > 0.0)
                .or_else(|| {
                    let pack = r.get::<Option<f64>, _>("pack_size")?;
                    matches!(
                        r.get::<Option<String>, _>("size_unit").as_deref(),
                        Some("g") | Some("ml")
                    )
                    .then_some(pack)
                    .filter(|p| *p > 0.0)
                });
            let per100 = grams.map(|g| (unit as f64 * 100.0 / g).round() as i64);
            json!({
                "unit_price_cents": unit,
                "qty": r.get::<Option<i32>,_>("qty_count"),
                "price_per_100": per100,
                "observed_at": r.get::<String,_>("observed_at"),
                "source": r.get::<String,_>("source"),
                "store": r.get::<String,_>("store"),
                "chain": r.get::<String,_>("chain"),
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

// ---------- fridge & shelf life ----------

#[derive(Deserialize)]
pub struct FridgeReq {
    pub product_id: i64,
    pub qty: Option<i32>,
    /// override; otherwise product override -> category default -> 14 days
    pub expires_in_days: Option<i32>,
    pub acquired_at: Option<String>, // YYYY-MM-DD
}

pub async fn fridge_add(
    State(st): State<AppState>,
    authed: Authed,
    Json(f): Json<FridgeReq>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let days = match f.expires_in_days {
        Some(d) => d,
        None => {
            let row = sqlx::query_as::<_, (i32,)>(
                "SELECT COALESCE(
                    (SELECT o.days FROM shelf_life_overrides o WHERE o.product_id=$1),
                    (SELECT d.days FROM shelf_life_defaults d JOIN products p ON p.category=d.category WHERE p.id=$1),
                    14)",
            )
            .bind(f.product_id)
            .fetch_one(&st.pool)
            .await?;
            row.0
        }
    };
    let acquired: NaiveDate = match &f.acquired_at {
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|_| AppError::BadRequest("acquired_at must be YYYY-MM-DD".into()))?,
        None => chrono::Local::now().date_naive(),
    };
    let row = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO fridge_items (user_id, product_id, qty, acquired_at, expires_at)
         VALUES ($1,$2,$3,$4,$5) RETURNING id",
    )
    .bind(authed.user_id)
    .bind(f.product_id)
    .bind(f.qty.unwrap_or(1))
    .bind(acquired)
    .bind(acquired + chrono::Duration::days(days as i64))
    .fetch_one(&st.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": row.0, "expires_in_days": days })),
    ))
}

pub async fn fridge_expiring(
    State(st): State<AppState>,
    authed: Authed,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult<Json<serde_json::Value>> {
    let days: i32 = q.get("days").and_then(|d| d.parse().ok()).unwrap_or(7);
    let rows = sqlx::query(
        "SELECT f.id, f.product_id, p.name, f.qty, f.expires_at, (f.expires_at - CURRENT_DATE) AS days_left
         FROM fridge_items f JOIN products p ON p.id=f.product_id
         WHERE f.user_id=$1 AND f.consumed_at IS NULL AND f.expires_at <= CURRENT_DATE + $2::int
         ORDER BY f.expires_at",
    )
    .bind(authed.user_id)
    .bind(days)
    .fetch_all(&st.pool)
    .await?;
    let out: Vec<_> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<i64,_>("id"),
                "product_id": r.get::<i64,_>("product_id"),
                "product": r.get::<String,_>("name"),
                "qty": r.get::<i32,_>("qty"),
                "days_left": r.get::<i32,_>("days_left"),
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

// ---------- shopping lists & plans ----------

#[derive(Deserialize)]
pub struct ListReq {
    pub name: String,
}

pub async fn create_list(
    State(st): State<AppState>,
    authed: Authed,
    Json(l): Json<ListReq>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let row = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO shopping_lists (user_id, name) VALUES ($1,$2) RETURNING id",
    )
    .bind(authed.user_id)
    .bind(&l.name)
    .fetch_one(&st.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": row.0 }))))
}

#[derive(Deserialize)]
pub struct ListItemReq {
    pub product_id: i64,
    pub qty: Option<i32>,
    pub note: Option<String>,
}

pub async fn add_list_item(
    State(st): State<AppState>,
    _authed: Authed,
    Path(list_id): Path<i64>,
    Json(it): Json<ListItemReq>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let row = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO shopping_list_items (list_id, product_id, qty, note) VALUES ($1,$2,$3,$4) RETURNING id",
    )
    .bind(list_id)
    .bind(it.product_id)
    .bind(it.qty.unwrap_or(1))
    .bind(&it.note)
    .fetch_one(&st.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": row.0 }))))
}

/// Greedy best-price plan: assign each list item to the store with the cheapest
/// known receipt price, then order the resulting stops. v1 uses most-recent-price
/// rather than normalized unit price until product sizes are curated.
pub async fn build_plan(
    State(st): State<AppState>,
    _authed: Authed,
    Path(list_id): Path<i64>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let rows = sqlx::query(
        "SELECT DISTINCT ON (i.product_id) i.product_id, p.name, i.qty,
                o.unit_price_cents, o.store_id, s.name AS store,
                o.qty_grams::float8 AS qty_grams, p.pack_size::float8 AS pack_size, p.size_unit
         FROM shopping_list_items i
         JOIN products p ON p.id = i.product_id
         JOIN price_observations o ON o.product_id = i.product_id
         JOIN stores s ON s.id = o.store_id
         WHERE i.list_id=$1
         ORDER BY i.product_id,
             CASE
               WHEN o.qty_grams IS NOT NULL AND o.qty_grams > 0
                 THEN o.unit_price_cents::float8 / (o.qty_grams / 100.0)
               WHEN p.size_unit IN ('g','ml') AND p.pack_size IS NOT NULL AND p.pack_size > 0
                 THEN o.unit_price_cents::float8 / (p.pack_size / 100.0)
               ELSE o.unit_price_cents::float8
             END ASC,
             o.observed_at DESC",
    )
    .bind(list_id)
    .fetch_all(&st.pool)
    .await?;
    let missing = sqlx::query(
        "SELECT p.name FROM shopping_list_items i JOIN products p ON p.id=i.product_id
         WHERE i.list_id=$1 AND NOT EXISTS (
            SELECT 1 FROM price_observations o WHERE o.product_id = i.product_id)",
    )
    .bind(list_id)
    .fetch_all(&st.pool)
    .await?;

    if rows.is_empty() {
        return Err(AppError::Unprocessable(
            "no priced observations for this list".into(),
        ));
    }
    let mut tx = st.pool.begin().await?;
    let plan =
        sqlx::query_as::<_, (i64,)>("INSERT INTO trip_plans (list_id) VALUES ($1) RETURNING id")
            .bind(list_id)
            .fetch_one(&mut *tx)
            .await?;
    let mut total = 0i64;
    let mut stop_order: Vec<i64> = Vec::new();
    let mut items_out = Vec::new();
    for r in &rows {
        let store_id: i64 = r.get("store_id");
        if !stop_order.contains(&store_id) {
            stop_order.push(store_id);
        }
        let qty: i32 = r.get("qty");
        let price: i64 = r.get("unit_price_cents");
        total += price * qty as i64;
        let grams = r
            .get::<Option<f64>, _>("qty_grams")
            .filter(|g| *g > 0.0)
            .or_else(|| {
                let pack = r.get::<Option<f64>, _>("pack_size")?;
                matches!(
                    r.get::<Option<String>, _>("size_unit").as_deref(),
                    Some("g") | Some("ml")
                )
                .then_some(pack)
                .filter(|p| *p > 0.0)
            });
        let per100 = grams.map(|g| (price as f64 * 100.0 / g).round() as i64);
        sqlx::query(
            "INSERT INTO plan_items (plan_id, product_id, store_id, qty, unit_price_cents)
             VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(plan.0)
        .bind(r.get::<i64, _>("product_id"))
        .bind(store_id)
        .bind(qty)
        .bind(price)
        .execute(&mut *tx)
        .await?;
        items_out.push(json!({
            "product": r.get::<String,_>("name"),
            "store": r.get::<String,_>("store"),
            "qty": qty,
            "unit_price_cents": price,
            "price_per_100": per100,
        }));
    }
    for (pos, store_id) in stop_order.iter().enumerate() {
        sqlx::query("INSERT INTO plan_stops (plan_id, store_id, position) VALUES ($1,$2,$3)")
            .bind(plan.0)
            .bind(store_id)
            .bind(pos as i32)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE trip_plans SET total_cents=$1 WHERE id=$2")
        .bind(total)
        .bind(plan.0)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let missing_names: Vec<String> = missing.iter().map(|r| r.get("name")).collect();
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "plan_id": plan.0,
            "total_cents": total,
            "stops": stop_order.len(),
            "items": items_out,
            "unpriced": missing_names,
        })),
    ))
}

// ---------- shelf-tag price integrity (advertised vs actually charged) ----------

#[derive(Deserialize)]
pub struct PriceCheckReq {
    pub store_id: i64,
    pub trip_id: Option<i64>,
    pub product_id: Option<i64>,
    pub upc: Option<String>,
    pub advertised_cents: i64,
    pub confidence: Option<f32>,
}

pub async fn create_price_check(
    State(st): State<AppState>,
    authed: Authed,
    Json(p): Json<PriceCheckReq>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    // resolve product via direct id or UPC -> barcode_links
    let product_id = match (p.product_id, &p.upc) {
        (Some(id), _) => Some(id),
        (None, Some(upc)) => {
            sqlx::query_as::<_, (i64,)>("SELECT product_id FROM barcode_links WHERE upc=$1")
                .bind(upc)
                .fetch_optional(&st.pool)
                .await
                .map(|r| r.map(|x| x.0))?
        }
        _ => None,
    };
    let row = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO price_checks (user_id, store_id, trip_id, product_id, upc, advertised_cents, confidence)
         VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id",
    )
    .bind(authed.user_id)
    .bind(p.store_id)
    .bind(p.trip_id)
    .bind(product_id)
    .bind(&p.upc)
    .bind(p.advertised_cents)
    .bind(p.confidence)
    .fetch_one(&st.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": row.0, "resolved_product_id": product_id })),
    ))
}

/// Compare every shelf-tag price recorded for a trip against what the receipt
/// actually charged at that store. Flags: MATCH / OVERCHARGE (register > shelf) /
/// UNDERCHARGE (register < shelf, e.g. expired promo was never taken down).
pub async fn trip_price_integrity(
    State(st): State<AppState>,
    _authed: Authed,
    Path(trip_id): Path<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    let rows = sqlx::query(
        "SELECT pc.id AS check_id, pc.advertised_cents, p.name AS product,
                ri.unit_price_cents AS charged_cents
         FROM price_checks pc
         JOIN receipts r ON r.trip_id = $1
         JOIN receipt_items ri ON ri.receipt_id = r.id AND ri.unit_price_cents IS NOT NULL
         LEFT JOIN products p ON p.id = pc.product_id
         WHERE pc.trip_id = $1 AND pc.product_id = ri.product_id",
    )
    .bind(trip_id)
    .fetch_all(&st.pool)
    .await?;
    let unmatched: Vec<String> = sqlx::query(
        "SELECT COALESCE(p.name, pc.upc, 'unknown product') FROM price_checks pc
         LEFT JOIN products p ON p.id = pc.product_id
         WHERE pc.trip_id=$1 AND NOT EXISTS (
             SELECT 1 FROM receipts r JOIN receipt_items ri ON ri.receipt_id=r.id
             WHERE r.trip_id=$1 AND ri.product_id = pc.product_id)",
    )
    .bind(trip_id)
    .fetch_all(&st.pool)
    .await?
    .iter()
    .map(|r| r.get::<String, _>("name"))
    .collect();
    let verdicts: Vec<_> = rows
        .iter()
        .map(|r| {
            let adv: i64 = r.get("advertised_cents");
            let charged: i64 = r.get("charged_cents");
            let verdict = if adv == charged {
                "MATCH"
            } else if charged > adv {
                "OVERCHARGE"
            } else {
                "UNDERCHARGE"
            };
            json!({
                "check_id": r.get::<i64,_>("check_id"),
                "product": r.get::<Option<String>,_>("product"),
                "advertised_cents": adv,
                "charged_cents": charged,
                "verdict": verdict,
            })
        })
        .collect();
    Ok(Json(
        json!({ "verdicts": verdicts, "unmatched_checks": unmatched }),
    ))
}

// ---------- local VLM receipt parsing (opt-in quality path) ----------
//
// The privacy stance holds: the image goes to the user's OWN Strata server
// on their LAN (VLM_URL in .env), is never persisted here, and the model's
// answer is a CANDIDATE — the same arithmetic checksums gate it that gate
// the OCR templates. Model proposes, arithmetic disposes.

#[derive(Deserialize)]
pub struct VlmReq {
    pub image_b64: String,
    pub mime: Option<String>,
}

const VLM_SYSTEM: &str = r#"You are a receipt parser. Output ONLY a JSON object — no prose, no markdown fences.
Schema (ALL money values are INTEGER CENTS):
{"chain":"costco"|"trader_joes"|"fresh_thyme"|"unknown",
 "store_name":string|null,"store_addr":string|null,"store_ref":string|null,
 "occurred_at":"YYYY-MM-DDTHH:MM:SS"|null,
 "txn_ref":string|null,
 "lines":[{"seq":1,"raw_name":"exact printed name","store_item_code":string|null,
   "discount_ref":null,"qty_count":int|null,"qty_grams":number|null,
   "unit_basis":"EA"|"LB"|"G"|"OZ"|null,"unit_price_cents":int|null,
   "line_total_cents":int,"savings_cents":int|null,"flags":[],
   "department":string|null,"confidence":0.9}],
 "subtotal_cents":int|null,"tax_cents":int|null,"tax_rate_milli_pct":int|null,
 "total_cents":int|null,
 "total_source":"printed"|"inferred_from_payment"|"inferred_from_subtotal_tax"|"missing",
 "discount_total_cents":int|null,"item_count":int|null,
 "issues":[],"parser_version":"vlm","raw_text":""}
Rules:
- Transcribe EVERY printed item line; copy raw_name exactly as printed.
- Convert weights to grams (1 lb = 453.59 g, 1 oz = 28.35 g).
- repeated identical item lines: ONE line with qty_count = repetition count.
- If the printed total is crossed out or unreadable, infer it from the payment block
  (card/debit amount) and set total_source to "inferred_from_payment".
- tax_rate_milli_pct: percent × 1000 (9.025% -> 9025).
- Only item lines; do NOT include payment/tender/subtotal/tax as lines.
- Never invent values; use null when not visible. Guess nothing.
- NEVER include membership numbers, card numbers or account numbers anywhere."#;

/// Drop digit runs of 12+ (membership/PAN-shaped) from free-text fields,
/// keeping chain item codes (<= 8 digits) intact.
fn scrub_long_digits(s: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    for c in s.chars().chain(std::iter::once('\0')) {
        if c.is_ascii_digit() {
            run.push(c);
        } else {
            if run.len() >= 12 {
                out.push_str("[REDACTED]");
            } else {
                out.push_str(&run);
            }
            run.clear();
            if c != '\0' {
                out.push(c);
            }
        }
    }
    out
}

pub async fn vlm_parse(
    State(_st): State<AppState>,
    _authed: Authed,
    Json(req): Json<VlmReq>,
) -> ApiResult<Json<ParsedReceipt>> {
    let base = std::env::var("VLM_URL").map_err(|_| {
        AppError::Internal(
            "VLM_URL not set — add it to .env (e.g. http://127.0.0.1:8079/v1)".into(),
        )
    })?;
    let key = std::env::var("VLM_API_KEY").unwrap_or_default();
    let model = std::env::var("VLM_MODEL").unwrap_or_else(|_| "qwen3.8-flash-next".into());
    let mime = req.mime.clone().unwrap_or_else(|| "image/jpeg".into());
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));

    let payload = json!({
        "model": model,
        "temperature": 0.0,
        "max_tokens": 4000,
        "messages": [
            { "role": "system", "content": VLM_SYSTEM },
            { "role": "user", "content": [
                { "type": "text", "text": "Extract this grocery receipt to the JSON schema." },
                { "type": "image_url", "image_url": { "url": format!("data:{mime};base64,{}", req.image_b64) } }
            ] }
        ]
    });

    let raw = tokio::task::spawn_blocking(move || -> Result<String, AppError> {
        let agent = ureq::AgentBuilder::new()
            .timeout_read(std::time::Duration::from_secs(300))
            .timeout_connect(std::time::Duration::from_secs(10))
            .build();
        let resp = agent
            .post(&url)
            .set("Authorization", &format!("Bearer {key}"))
            .set("Content-Type", "application/json")
            .send_string(&payload.to_string())
            .map_err(|e| AppError::Internal(format!("VLM request to {url} failed: {e}")))?;
        let text = resp
            .into_string()
            .map_err(|e| AppError::Internal(format!("VLM body unreadable: {e}")))?;
        let body: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| AppError::Internal(format!("VLM response was not JSON: {e}")))?;
        body["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "VLM response missing content: {}",
                    &body.to_string()[..500.min(body.to_string().len())]
                ))
            })
    })
    .await
    .map_err(|e| AppError::Internal(format!("vlm task join: {e}")))??;

    let stripped = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
        .to_string();
    // Model never owns issues or parser metadata — strip them before typed
    // parse so junk variants in those fields can't reject an otherwise good parse.
    let mut value: serde_json::Value = serde_json::from_str(&stripped).map_err(|e| {
        let head: String = stripped.chars().take(300).collect();
        AppError::Unprocessable(format!(
            "model output not valid JSON ({e}) — model said: {head}"
        ))
    })?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("issues".into(), serde_json::json!([]));
        obj.insert("parser_version".into(), serde_json::json!("vlm"));
        obj.insert("raw_text".into(), serde_json::json!(""));
    }
    let mut receipt: ParsedReceipt = serde_json::from_value(value).map_err(|e| {
        let head: String = stripped.chars().take(300).collect();
        AppError::Unprocessable(format!(
            "model JSON doesn't fit receipt schema ({e}) — model said: {head}"
        ))
    })?;

    // defense in depth against PII regardless of model behavior
    for s in [
        &mut receipt.txn_ref,
        &mut receipt.store_ref,
        &mut receipt.store_name,
    ] {
        if let Some(v) = s {
            *v = scrub_long_digits(v);
        }
    }
    for l in &mut receipt.lines {
        l.raw_name = scrub_long_digits(&l.raw_name);
    }
    receipt.raw_text = String::new(); // OCR text stays on the model's transcript; nothing persisted here
    receipt.parser_version = format!("vlm:{model}");
    receipt.issues = grocer_parser::verify(&receipt);
    if receipt.lines.is_empty() {
        receipt.issue(
            grocer_parser::IssueKind::ParsedNothing,
            "model returned no item lines",
        );
    }
    tracing::info!(lines = receipt.lines.len(), chain = ?receipt.chain, "vlm parse");
    Ok(Json(receipt))
}
