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

/// OCR-tolerant chain detection: score distinctive markers per chain instead
/// of trusting the header line (real OCR mangles logos/headers — "COSTCO
/// WHOLESALE" came through as garbage — while the footer trio Whse:/Trm:/Trn:
/// and structural lines survive). Highest score wins; >=2 required.
pub fn detect(text: &str) -> Chain {
    let u = text.to_ascii_uppercase().replace([' ', '\t'], " ");
    let u = u.as_str();
    let score = |markers: &[(&str, u8)]| -> u8 {
        markers
            .iter()
            .filter(|(m, _)| u.contains(m))
            .map(|(_, p)| *p)
            .sum()
    };
    let candidates = [
        (
            Chain::Costco,
            score(&[
                ("COSTCO", 5),
                ("WHSE:", 2),
                ("TRM:", 2),
                ("TRN:", 2),
                ("INSTANT SAVINGS", 2),
                ("TOTAL NUMBER OF ITEMS SOLD", 2),
                ("MEMBER #", 1),
            ]),
        ),
        (
            Chain::TraderJoes,
            score(&[
                ("TRADER JOE", 5),
                ("TRADE JOE", 5),
                ("PJ TRADING", 3),
                ("TRADER JOE’S", 5),
            ]),
        ),
        (
            Chain::FreshThyme,
            score(&[
                ("FRESH THYME", 5),
                ("THYME MARKET", 5),
                ("YOU SAVED", 2),
                ("MIN 3", 1),
            ]),
        ),
    ];
    candidates
        .iter()
        .max_by_key(|(_, s)| *s)
        .filter(|(_, s)| *s >= 2)
        .map(|(c, _)| *c)
        .unwrap_or(Chain::Unknown)
}

/// Detect the chain and parse. `text` is the OCR'd receipt text (line-oriented).
pub fn parse(text: &str) -> ParsedReceipt {
    let chain = detect(text);
    let mut receipt = match chain {
        Chain::FreshThyme => fresh_thyme::parse(text),
        Chain::TraderJoes => trader_joes::parse(text),
        Chain::Costco => costco::parse(text),
        Chain::Unknown => model::ParsedReceipt::new(),
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
