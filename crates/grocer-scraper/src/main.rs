use grocer_scraper::{ChainCatalog, FixtureChain};
use sqlx::PgPool;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://grocer:grocer@localhost:5435/grocer".into());

    let chain: Box<dyn ChainCatalog> = match args.get(1).map(|s| s.as_str()) {
        Some("fixture") => Box::new(FixtureChain {
            url: args.get(2).cloned().unwrap_or_else(|| {
                eprintln!("usage: grocer-scraper fixture <url-or-file://path>");
                std::process::exit(2);
            }),
        }),
        // Real chains plug in here once their parser modules + fixtures exist:
        // Some("cub") => Box::new(chains::Cub::new(store_zip)), ...
        other => {
            eprintln!("unknown chain: {:?} (available: fixture)", other);
            std::process::exit(2);
        }
    };

    let html = if chain.fetch_url().starts_with("file://") {
        std::fs::read_to_string(chain.fetch_url().trim_start_matches("file://"))?
    } else {
        // polite single GET, one request per cron tick per chain
        let resp = ureq::get(&chain.fetch_url())
            .set("user-agent", &chain.user_agent())
            .timeout(std::time::Duration::from_secs(30))
            .call()?;
        resp.into_string()?
    };

    let prices = chain.parse(&html);
    println!(
        "parsed {} candidate prices for chain '{}'",
        prices.len(),
        chain.chain_key()
    );

    let pool = PgPool::connect(&db_url).await?;
    let chain_row: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM chains WHERE parser_key=$1 OR name=$1")
            .bind(chain.chain_key())
            .fetch_optional(&pool)
            .await?;
    let Some(chain_id) = chain_row.map(|r| r.0) else {
        println!(
            "chain '{}' not registered in DB; nothing to do",
            chain.chain_key()
        );
        return Ok(());
    };

    let scraped_at = grocer_scraper::now_iso();
    let (mut inserted, mut skipped_receipt, mut skipped_nostore) = (0, 0, 0);
    for p in &prices {
        let store: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM stores WHERE chain_id=$1 AND store_ref=$2")
                .bind(chain_id)
                .bind(&p.store_ref)
                .fetch_optional(&pool)
                .await?;
        let Some(store_id) = store.map(|s| s.0) else {
            skipped_nostore += 1;
            continue;
        };

        // Resolve product: barcode_links via UPC, else leave product-less.
        let product_id: Option<i64> = match &p.upc {
            Some(upc) => {
                sqlx::query_as::<_, (i64,)>("SELECT product_id FROM barcode_links WHERE upc=$1")
                    .bind(upc)
                    .fetch_optional(&pool)
                    .await?
                    .map(|r| r.0)
            }
            None => None,
        };

        // Receipts outrank scrapers: if a receipt observed this store+product
        // within 30 days, the scraped price is dropped. ISO text compares
        // lexicographically, so a plain string cutoff works.
        let beaten_by_receipt: Option<(i64,)> = if let Some(pid) = product_id {
            let cutoff = (chrono::Utc::now() - chrono::Duration::days(30))
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string();
            sqlx::query_as(
                "SELECT 1::int8 FROM price_observations o
                 WHERE o.store_id=$1 AND o.product_id=$2 AND o.source='receipt'
                   AND o.observed_at >= $3
                 LIMIT 1",
            )
            .bind(store_id)
            .bind(pid)
            .bind(cutoff)
            .fetch_optional(&pool)
            .await?
        } else {
            None
        };
        if product_id.is_some() && beaten_by_receipt.is_some() {
            skipped_receipt += 1;
            continue;
        }

        sqlx::query(
            "INSERT INTO price_observations (store_id, chain_id, product_id, raw_name,
                qty_count, unit_price_cents, observed_at, source)
             VALUES ($1,$2,$3,$4,1,$5,$6,'scraper')",
        )
        .bind(store_id)
        .bind(chain_id)
        .bind(product_id)
        .bind(&p.raw_name)
        .bind(p.unit_price_cents)
        .bind(&scraped_at)
        .execute(&pool)
        .await?;
        inserted += 1;
    }
    println!(
        "inserted {inserted}; skipped {skipped_receipt} (recent receipt is truth), \
         {skipped_nostore} (store not registered)"
    );
    Ok(())
}
