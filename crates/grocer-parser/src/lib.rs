//! grocer-parser: deterministic per-chain receipt template parsers with arithmetic
//! self-validation. Runs on the server for verification and (phase 2) in the browser via WASM.
//!
//! Design rule: OCR text is *candidate* text. We classify lines against chain templates,
//! ignore what we don't understand, and prove the result arithmetically
//! (sum lines == subtotal, subtotal+tax == total, qty*unit ~= line total). Anything that
//! fails a check is reported in `issues` for on-device human review.

mod chains;
mod dateparse;
mod model;
mod money;
mod validate;

pub use model::*;

use chains::{costco, fresh_thyme, trader_joes};

/// Detect the chain and parse. `text` is the OCR'd receipt text (line-oriented).
pub fn parse(text: &str) -> ParsedReceipt {
    let upper = text.to_ascii_uppercase();
    let (chain, mut receipt) = if upper.contains("FRESH THYME") {
        (Chain::FreshThyme, fresh_thyme::parse(text))
    } else if upper.contains("TRADER JOE") {
        (Chain::TraderJoes, trader_joes::parse(text))
    } else if upper.contains("COSTCO") {
        (Chain::Costco, costco::parse(text))
    } else {
        (Chain::Unknown, model::ParsedReceipt::new())
    };
    receipt.chain = chain;
    receipt.raw_text = crate::chains::mask_pii(text);
    validate::validate(&mut receipt);
    receipt
}

/// Re-verify a receipt parsed on a device: recompute all checksums from the item lines and
/// report any inconsistency. Used server-side before accepting an ingest.
pub fn verify(parsed: &ParsedReceipt) -> Vec<Issue> {
    let mut r = parsed.clone();
    // Recompute from scratch: drop client-claimed totals we can derive ourselves.
    validate::validate(&mut r);
    let mut out = r.issues;
    // Cross-check the client's own claimed totals against the lines.
    let sum: i64 = parsed.lines.iter().map(|l| l.line_total_cents).sum();
    if let Some(sub) = parsed.subtotal_cents {
        if sub != sum {
            out.push(Issue {
                kind: IssueKind::SumMismatch,
                detail: format!("claimed subtotal {sub} != sum of lines {sum}"),
            });
        }
    }
    if let (Some(sub), Some(tax), Some(tot)) =
        (parsed.subtotal_cents, parsed.tax_cents, parsed.total_cents)
    {
        if sub + tax != tot {
            out.push(Issue {
                kind: IssueKind::SumMismatch,
                detail: format!("subtotal {sub} + tax {tax} != total {tot}"),
            });
        }
    }
    out
}
