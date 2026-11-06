use crate::chains::{note_pii, pop_flags};
use crate::dateparse::find_datetime;
use crate::model::*;
use crate::money::{find_int, parse_money, tail_money};

/// Costco template. Hazards proven by the real corpus (5 photos, same txn):
/// item codes, E/A flags, coupon line `0000372982 /1961075 5.00-`, redacted TOTAL,
/// double-printed date lines, membership number (PII), repeated lines for qty>1.
pub fn parse(text: &str) -> ParsedReceipt {
    let mut r = ParsedReceipt::new();
    let mut payment_amount: Option<i64> = None;
    let mut member_line_seen = false;

    for raw in text.lines() {
        let l = raw.trim();
        if l.is_empty() {
            continue;
        }
        let up = l.to_ascii_uppercase();

        if up.contains("COSTCO WHOLESALE") {
            r.store_name = Some("Costco Wholesale".into());
            continue;
        }
        if up.contains("MEMBER") {
            member_line_seen = true;
            continue;
        }
        if let Some(i) = l.find("BD #") {
            let num: String = l[i + 4..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !num.is_empty() {
                r.store_ref = Some(num);
            }
            continue;
        }
        // Date lines (appears in payment block and footer; first wins).
        if let Some(dt) = find_datetime(l) {
            if r.occurred_at.is_none() {
                r.occurred_at = Some(dt);
            }
            continue;
        }
        if up.starts_with("SUBTOTAL") {
            if let Some(v) = tail_money(l) {
                r.subtotal_cents = Some(v);
            }
            continue;
        }
        if up.starts_with("TAX A") && l.contains('%') {
            // "TAX A 9.025% 3.32"
            if let Some(v) = tail_money(l) {
                r.tax_cents = Some(v);
            }
            if let Some(rate) = l
                .split_whitespace()
                .find(|t| t.ends_with('%'))
                .and_then(|t| t.trim_end_matches('%').parse::<f64>().ok())
            {
                r.tax_rate_milli_pct = Some((rate * 1000.0).round() as i64);
            }
            continue;
        }
        if up.starts_with("TOTAL TAX") {
            if r.tax_cents.is_none() {
                r.tax_cents = tail_money(l);
            }
            continue;
        }
        if up.starts_with("TAX ") && !l.contains('%') {
            if r.tax_cents.is_none() {
                r.tax_cents = tail_money(l);
            }
            continue;
        }
        if l.starts_with("*** TOTAL") || up.starts_with("**** TOTAL") {
            match tail_money(l) {
                Some(v) => {
                    r.total_cents = Some(v);
                    r.total_source = TotalSource::Printed;
                }
                None => r.issue(
                    IssueKind::TotalMissing,
                    "printed TOTAL line unreadable (redacted/damaged)",
                ),
            }
            continue;
        }
        if up.contains("TOTAL NUMBER OF ITEMS SOLD") {
            r.item_count = find_int(l);
            continue;
        }
        if up.starts_with("INSTANT SAVINGS") {
            if let Some(v) = tail_money(l) {
                r.discount_total_cents = Some(-v.abs());
            }
            continue;
        }
        if up.starts_with("AMOUNT") {
            if let Some(v) = tail_money(l) {
                payment_amount = Some(v);
            }
            continue;
        }
        if up.starts_with("EFT/DEBIT") || up.starts_with("CREDIT") || up.starts_with("DEBIT") {
            if let Some(v) = tail_money(l) {
                payment_amount.get_or_insert(v);
            }
            continue;
        }
        if up.starts_with("CHANGE") {
            continue;
        }
        if up.contains("TRAN ID") {
            // take the longest digit run as the transaction reference
            let mut run = String::new();
            let mut best = String::new();
            for c in l.chars() {
                if c.is_ascii_digit() {
                    run.push(c);
                } else {
                    if run.len() > best.len() {
                        best = run.clone();
                    }
                    run.clear();
                }
            }
            if run.len() > best.len() {
                best = run;
            }
            if !best.is_empty() {
                r.txn_ref = Some(best);
            }
            continue;
        }
        // Item / coupon line: 'E' + all-digit code + ...
        if l.starts_with('E') {
            let rest = l[1..].trim();
            let toks: Vec<&str> = rest.split_whitespace().collect();
            if toks.len() >= 3 && !toks[0].is_empty() && toks[0].chars().all(|c| c.is_ascii_digit())
            {
                let code = toks[0].to_string();
                let mut toks = toks[1..].to_vec();
                let flags = pop_flags(&mut toks, &['A', 'E', 'N', 'F', 'X', 'x']);
                let total = match parse_money(toks[toks.len() - 1]) {
                    Some(v) => {
                        toks.pop();
                        v
                    }
                    None => continue, // unparseable tail: treat as junk, don't abort the parse
                };
                let name_toks = toks;
                let coupon_ref = name_toks
                    .iter()
                    .find(|t| t.starts_with('/'))
                    .map(|t| t.trim_start_matches('/').to_string());
                let raw_name: String = name_toks
                    .iter()
                    .filter(|t| !t.starts_with('/'))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                let is_coupon = coupon_ref.is_some();
                let line = ItemLine {
                    seq: r.lines.len() + 1,
                    raw_name: if raw_name.is_empty() && coupon_ref.is_some() {
                        format!("COUPON /{}", coupon_ref.as_ref().unwrap())
                    } else {
                        raw_name
                    },
                    store_item_code: Some(code),
                    discount_ref: coupon_ref,
                    // A coupon's face value is not a unit price — don't model it as one.
                    qty_count: if is_coupon { None } else { Some(1) },
                    qty_grams: None,
                    unit_basis: if is_coupon { None } else { Some("EA".into()) },
                    unit_price_cents: if is_coupon { None } else { Some(total.abs()) },
                    line_total_cents: total,
                    savings_cents: None,
                    flags,
                    department: None,
                    confidence: 0.95,
                };
                r.push_line(line);
                continue;
            }
        }
        // everything else: header/address/junk — intentionally ignored
    }

    finish(r, payment_amount, member_line_seen)
}

fn finish(
    mut r: ParsedReceipt,
    payment_amount: Option<i64>,
    member_line_seen: bool,
) -> ParsedReceipt {
    if member_line_seen {
        note_pii(&mut r);
    }
    if r.total_cents.is_none() {
        if let Some(p) = payment_amount {
            r.total_cents = Some(p);
            r.total_source = TotalSource::InferredFromPayment;
            r.issue(
                IssueKind::TotalInconsistent,
                "total recovered from payment block; verified against subtotal+tax",
            );
        } else if let (Some(s), Some(t)) = (r.subtotal_cents, r.tax_cents) {
            r.total_cents = Some(s + t);
            r.total_source = TotalSource::InferredFromSubtotalTax;
        }
    }
    r
}
