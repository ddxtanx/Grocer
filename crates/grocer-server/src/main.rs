mod api;
mod auth;
mod error;

use axum::routing::{get, post};
use axum::Router;
use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pool: PgPool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env from the working directory (repo root) if present; real env
    // vars always win over .env values. See .env.example.
    dotenvy::dotenv().ok();

    // Credentials never live in source: DATABASE_URL must come from the real
    // environment or a .env file next to the working directory.
    let db_url = std::env::var("DATABASE_URL").map_err(|_| {
        "DATABASE_URL not set — copy .env.example to .env in the repo root, or export it"
            .to_string()
    })?;
    let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8787);

    let pool = PgPool::connect(&db_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let static_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/static");
    let app = Router::new()
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        .route("/api/stores", get(api::list_stores))
        .route("/api/trips", post(api::create_trip).get(api::my_trips))
        .route(
            "/api/receipts",
            post(api::ingest_receipt).get(api::my_receipts),
        )
        .route("/api/receipts/{id}", get(api::receipt_detail))
        .route("/api/parse_text", post(api::parse_text))
        .route(
            "/api/products",
            post(api::create_product).get(api::search_products),
        )
        .route("/api/items/unlinked", get(api::unlinked_items))
        .route("/api/receipt_items/{id}/link", post(api::link_receipt_item))
        .route("/api/barcodes", post(api::link_barcode))
        .route("/api/prices", get(api::price_history))
        .route("/api/fridge", post(api::fridge_add))
        .route("/api/fridge/expiring", get(api::fridge_expiring))
        .route("/api/lists", post(api::create_list))
        .route("/api/lists/{id}/items", post(api::add_list_item))
        .route("/api/plans/{id}", post(api::build_plan))
        .route("/api/price-checks", post(api::create_price_check))
        .route(
            "/api/trips/{id}/price-integrity",
            get(api::trip_price_integrity),
        )
        .fallback_service(
            tower_http::services::ServeDir::new(static_dir).append_index_html_on_directories(true),
        )
        .with_state(AppState { pool });

    let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await?;
    println!("grocer-server listening on http://{host}:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
