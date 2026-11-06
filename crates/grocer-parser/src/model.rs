use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Chain {
    FreshThyme,
    TraderJoes,
    Costco,
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TotalSource {
    Printed,
    InferredFromPayment,
    InferredFromSubtotalTax,
    Missing,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    SumMismatch,
    LineMathMismatch,
    ItemCountMismatch,
    TaxMismatch,
    TotalMissing,
    TotalInconsistent,
    LowConfidenceLine,
    PiiMasked,
    ParsedNothing,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Issue {
    pub kind: IssueKind,
    pub detail: String,
}

/// A single parsed item line. All money is integer cents.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ItemLine {
    pub seq: usize,
    pub raw_name: String,
    /// Chain-internal SKU (Costco item code). Stable key for cross-scan product linking.
    pub store_item_code: Option<String>,
    /// For coupon/discount lines: the item code this discount refers to (e.g. `/1961075`).
    pub discount_ref: Option<String>,
    pub qty_count: Option<i32>,
    pub qty_grams: Option<f64>,
    /// Basis of the unit price: "EA", "LB", "G"…
    pub unit_basis: Option<String>,
    pub unit_price_cents: Option<i64>,
    pub line_total_cents: i64,
    /// Informational savings on this line (already netted into the price by some POSes).
    pub savings_cents: Option<i64>,
    pub flags: Vec<String>,
    pub department: Option<String>,
    pub confidence: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ParsedReceipt {
    pub chain: Chain,
    pub store_name: Option<String>,
    pub store_addr: Option<String>,
    pub store_ref: Option<String>,
    /// ISO 8601 local timestamp of the transaction, when recoverable.
    pub occurred_at: Option<String>,
    pub txn_ref: Option<String>,
    pub lines: Vec<ItemLine>,
    pub subtotal_cents: Option<i64>,
    pub tax_cents: Option<i64>,
    /// Tax rate in milli-percent: 9.025% -> 9025.
    pub tax_rate_milli_pct: Option<i64>,
    pub total_cents: Option<i64>,
    pub total_source: TotalSource,
    pub discount_total_cents: Option<i64>,
    pub item_count: Option<i32>,
    pub issues: Vec<Issue>,
    pub parser_version: String,
    /// PII-masked original OCR text (stored for re-parsing when templates improve).
    pub raw_text: String,
}

impl ParsedReceipt {
    pub fn new() -> Self {
        Self {
            chain: Chain::Unknown,
            store_name: None,
            store_addr: None,
            store_ref: None,
            occurred_at: None,
            txn_ref: None,
            lines: Vec::new(),
            subtotal_cents: None,
            tax_cents: None,
            tax_rate_milli_pct: None,
            total_cents: None,
            total_source: TotalSource::Missing,
            discount_total_cents: None,
            item_count: None,
            issues: Vec::new(),
            parser_version: env!("CARGO_PKG_VERSION").to_string(),
            raw_text: String::new(),
        }
    }

    pub fn push_line(&mut self, l: ItemLine) {
        self.lines.push(l);
    }

    pub fn issue(&mut self, kind: IssueKind, detail: impl Into<String>) {
        self.issues.push(Issue {
            kind,
            detail: detail.into(),
        });
    }
}
