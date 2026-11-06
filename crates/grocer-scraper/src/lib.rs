//! Scraper sidecar: polite, cron-able, per-chain catalog price fetcher.
//!
//! Design rules (from DESIGN.md):
//!   * Scraped prices are *expectations*. A receipt observation is truth:
//!     if a receipt observed this store's price for the same product within
//!     the last 30 days, the scraped value is dropped on insert.
//!   * One module per chain; parsing is pure and unit-tested against saved
//!     HTML fixtures so template drift is caught without hitting the network.
//!   * The sidecar runs on the server host on a schedule; nothing about it
//!     is visible to other users (crowd receipts already carry the truth).

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq)]
pub struct ScrapedPrice {
    pub store_ref: String,
    pub raw_name: String,
    pub upc: Option<String>,
    pub unit_price_cents: i64,
    pub pack_size: Option<f64>,
    pub size_unit: Option<String>,
}

pub trait ChainCatalog {
    fn chain_key(&self) -> &'static str;
    fn fetch_url(&self) -> String;
    fn user_agent(&self) -> String {
        "Mozilla/5.0 (compatible; grocer-selfhosted/1.0; +contact: owner configured this scraper)"
            .into()
    }
    /// Pure HTML -> prices. Must be deterministic and unit-tested against a
    /// fixture captured from the real site.
    fn parse(&self, html: &str) -> Vec<ScrapedPrice>;
}

// ---------- demo/reference chain: fixture-driven, exercised in tests ----------

pub struct FixtureChain {
    pub url: String,
}

impl ChainCatalog for FixtureChain {
    fn chain_key(&self) -> &'static str {
        "fixture"
    }
    fn fetch_url(&self) -> String {
        self.url.clone()
    }
    fn parse(&self, html: &str) -> Vec<ScrapedPrice> {
        // Minimal contract the real modules will share:
        // <div class="product" data-upc="..." data-size="500" data-unit="g" data-price="3.49">Name</div>
        parse_data_products(html)
    }
}

pub(crate) fn parse_data_products(html: &str) -> Vec<ScrapedPrice> {
    let mut out = Vec::new();
    for tag in attr_blocks(html) {
        let Some(price) = attr(&tag, "data-price").and_then(|p| money(&p)) else {
            continue;
        };
        out.push(ScrapedPrice {
            store_ref: attr(&tag, "data-store").unwrap_or_else(|| "online".into()),
            raw_name: inner_text(&tag),
            upc: attr(&tag, "data-upc"),
            unit_price_cents: price,
            pack_size: attr(&tag, "data-size").and_then(|s| s.parse().ok()),
            size_unit: attr(&tag, "data-unit"),
        });
    }
    out
}

fn attr_blocks(html: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut idx = 0;
    while let Some(rel) = html[idx..].find(r#"<div class="product""#) {
        let start = idx + rel;
        let Some(end) = html[start..].find("</div>") else {
            break;
        };
        blocks.push(html[start..start + end].to_string());
        idx = start + end + 6;
    }
    blocks
}

fn attr(block: &str, name: &str) -> Option<String> {
    let pat = format!(r#"{name}=""#);
    let i = block.find(&pat)? + pat.len();
    let j = block[i..].find('"')?;
    Some(block[i..i + j].to_string())
}

fn inner_text(block: &str) -> String {
    match block.rfind('>') {
        Some(i) => block[i + 1..].trim().to_string(),
        None => String::new(),
    }
}

fn money(s: &str) -> Option<i64> {
    let clean: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let v: f64 = clean.parse().ok()?;
    Some((v * 100.0).round() as i64)
}

pub fn now_iso() -> String {
    DateTime::<Utc>::from(Utc::now())
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
    <html><body>
      <div class="product" data-store="00905" data-upc="012345678901" data-size="454" data-unit="g" data-price="3.49">Organic Spinach</div>
      <div class="product" data-store="00905" data-price="2.19">Organic Bananas</div>
      <div class="promo-banner">ignore me</div>
      <div class="product" data-price="bogus">No Price</div>
    </body></html>"#;

    #[test]
    fn parses_fixture_products() {
        let p = FixtureChain { url: String::new() }.parse(FIXTURE);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].raw_name, "Organic Spinach");
        assert_eq!(p[0].unit_price_cents, 349);
        assert_eq!(p[0].upc.as_deref(), Some("012345678901"));
        assert_eq!(p[0].pack_size, Some(454.0));
        assert_eq!(p[1].raw_name, "Organic Bananas");
        assert_eq!(p[1].unit_price_cents, 219);
        assert_eq!(p[1].store_ref, "00905");
    }
}
