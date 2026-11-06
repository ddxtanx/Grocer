use crate::model::*;

/// Prove the parse arithmetically. A receipt is trusted when its own printed
/// figures agree with the line items.
pub fn validate(r: &mut ParsedReceipt) {
    if r.lines.is_empty() {
        r.issue(
            IssueKind::ParsedNothing,
            "no item lines matched the chain template",
        );
        return;
    }

    // Per-line math: qty x unit ~= line total (count-based lines only; weights can't
    // be checked without the printer's own rounding, coupons are exact by construction).
    let mut line_issues = Vec::new();
    for l in &r.lines {
        if l.discount_ref.is_some() {
            continue; // coupon face values aren't qty*unit products
        }
        if let (Some(q), Some(u)) = (l.qty_count, l.unit_price_cents) {
            if l.line_total_cents != 0 && (q as i64) * u != l.line_total_cents {
                line_issues.push(format!(
                    "'{}': {q} x {u} != {}",
                    l.raw_name, l.line_total_cents
                ));
            }
        }
    }
    for detail in line_issues {
        r.issue(IssueKind::LineMathMismatch, detail);
    }

    // Subtotal: compare to printed figure or synthesize from lines.
    let sum: i64 = r.lines.iter().map(|l| l.line_total_cents).sum();
    match r.subtotal_cents {
        Some(sub) => {
            if sub != sum {
                r.issue(
                    IssueKind::SumMismatch,
                    format!("printed subtotal {sub} != sum of lines {sum}"),
                );
            }
        }
        None => r.subtotal_cents = Some(sum),
    }

    // Total: printed total must equal subtotal + tax (tax defaults to 0 — Fresh Thyme
    // prints no tax line at all for all-food baskets).
    let tax = r.tax_cents.unwrap_or(0);
    let sub = r.subtotal_cents.unwrap();
    match r.total_cents {
        Some(tot) => {
            if tot != sub + tax {
                r.issue(
                    IssueKind::SumMismatch,
                    format!("total {tot} != subtotal {sub} + tax {tax}"),
                );
            }
        }
        None => {
            r.issue(IssueKind::TotalMissing, "no printed total recoverable");
        }
    }

    // Item count: number of units sold vs printed count (soft check).
    if let Some(cnt) = r.item_count {
        let qty: i32 = r
            .lines
            .iter()
            .filter(|l| l.discount_ref.is_none())
            .map(|l| l.qty_count.unwrap_or(1))
            .sum();
        if qty != cnt {
            r.issue(
                IssueKind::ItemCountMismatch,
                format!("printed item count {cnt} != summed qty {qty}"),
            );
        }
    }

    // Tax-rate cross-check when the receipt prints taxable flags (Costco `A` lines).
    if let (Some(tax), Some(rate)) = (r.tax_cents, r.tax_rate_milli_pct) {
        let taxable: i64 = r
            .lines
            .iter()
            .filter(|l| l.flags.iter().any(|f| f == "A"))
            .map(|l| l.line_total_cents)
            .sum();
        if taxable > 0 {
            let expected = (taxable as f64 * rate as f64 / 100_000.0).round() as i64;
            if (expected - tax).abs() > 5 {
                r.issue(
                    IssueKind::TaxMismatch,
                    format!("printed tax {tax} != rate*taxable {expected}"),
                );
            }
        }
    }
}
