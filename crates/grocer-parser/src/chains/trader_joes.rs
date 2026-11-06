use crate::chains::pop_flags;
use crate::dateparse::find_datetime;
use crate::model::*;
use crate::money::{find_int, parse_money, tail_money};

/// Trader Joe's template. Hazards from the real corpus: names truncated at ~22 chars,
/// qty>1 shown as repeated identical lines *and* `3 @ $2.49` sub-lines, footer-only date,
/// `Tax 7.47 @ 9.025% 0.67`, no subtotal line, dot-matrix noise.
pub fn parse(text: &str) -> ParsedReceipt {
    let mut r = ParsedReceipt::new();
    let mut payment_amount: Option<i64> = None;

    for raw in text.lines() {
        let l = raw.trim();
        if l.is_empty() {
            continue;
        }
        let up = l.to_ascii_uppercase();

        if up.contains("TRADER JOE") && !up.contains("SHOPPING") {
            r.store_name = Some("Trader Joe's".into());
            continue;
        }
        if up.contains("STORE #") {
            if let Some(i) = l.find('#') {
                let num: String = l[i + 1..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if !num.is_empty() {
                    r.store_ref = Some(num);
                }
            }
            continue;
        }
        if up.starts_with("DATE ") {
            if r.occurred_at.is_none() {
                r.occurred_at = find_datetime(l);
            }
            continue;
        }
        if up.starts_with("TRANS.") {
            r.txn_ref = find_int(l).map(|v| v.to_string());
            continue;
        }
        if up.starts_with("ITEMS IN TRANSACTION") {
            r.item_count = find_int(l);
            continue;
        }
        if up.starts_with("BALANCE TO PAY") {
            if let Some(v) = tail_money(l) {
                r.total_cents = Some(v);
                r.total_source = TotalSource::Printed;
            }
            continue;
        }
        if up.starts_with("TOTAL PURCHASE") {
            payment_amount = tail_money(l);
            continue;
        }
        if up.starts_with("TAX ") && l.contains('@') {
            // "Tax 7.47 @ 9.025% 0.67"
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
        // Quantity sub-line: "3 @ $2.49"
        {
            let toks: Vec<&str> = l.split_whitespace().collect();
            if toks.len() == 3 && toks[1] == "@" && !r.lines.is_empty() {
                if let (Some(q), Some(u)) = (toks[0].parse::<i32>().ok(), parse_money(toks[2])) {
                    let last = r.lines.last_mut().unwrap();
                    last.qty_count = Some(q);
                    last.unit_price_cents = Some(u);
                    last.unit_basis = Some("EA".into());
                    continue;
                }
            }
        }
        // Default item rule (TJ's lines are `NAME <truncated> PRICE`):
        // last token is money, at least one name token before it.
        {
            let mut toks: Vec<&str> = l.split_whitespace().collect();
            if toks.len() >= 2 {
                let flags = pop_flags(&mut toks, &['N', 'F', 'X', 'x']);
                if let Some(v) = toks.last().and_then(|t| parse_money(t)) {
                    toks.pop();
                    if toks.is_empty() {
                        continue;
                    }
                    let name: String = toks.join(" ");
                    r.push_line(ItemLine {
                        seq: r.lines.len() + 1,
                        raw_name: name,
                        store_item_code: None,
                        discount_ref: None,
                        qty_count: Some(1),
                        qty_grams: None,
                        unit_basis: Some("EA".into()),
                        unit_price_cents: Some(v),
                        line_total_cents: v,
                        savings_cents: None,
                        flags,
                        department: None,
                        confidence: 0.9,
                    });
                    continue;
                }
            }
        }
    }

    // Merge repeated identical lines (Costco-style qty by repetition): "MEATLESS KOREAN
    // BULGOGI 4.99" x3 -> qty 3.
    let mut merged: Vec<ItemLine> = Vec::new();
    for line in r.lines.drain(..) {
        if let Some(prev) = merged.last_mut() {
            // merge on name + unit price (line totals accumulate once merged,
            // so they can't be part of the key)
            if prev.raw_name == line.raw_name && prev.unit_price_cents == line.unit_price_cents {
                prev.qty_count = Some(prev.qty_count.unwrap_or(1) + line.qty_count.unwrap_or(1));
                prev.line_total_cents += line.line_total_cents;
                continue;
            }
        }
        merged.push(line);
    }
    r.lines = merged
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let mut l = l.clone();
            l.seq = i + 1;
            l
        })
        .collect();

    if r.total_cents.is_none() {
        if let Some(p) = payment_amount {
            r.total_cents = Some(p);
            r.total_source = TotalSource::InferredFromPayment;
        }
    }
    r
}
