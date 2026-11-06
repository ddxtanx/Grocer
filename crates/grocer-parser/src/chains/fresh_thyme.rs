use crate::dateparse::find_datetime;
use crate::model::*;
use crate::money::{find_int, parse_money, tail_money};

const DEPARTMENTS: &[&str] = &[
    "PRODUCE",
    "DAIRY",
    "GROCERY",
    "MEAT",
    "BAKERY",
    "FROZEN",
    "FLORAL",
    "SEAFOOD",
    "DELI",
    "BEER",
    "VITAMINS",
    "HOUSEHOLD",
    "SNACKS",
    "BEVERAGES",
    "BODY CARE",
    "NUTRITION",
    "OTHER",
];

fn is_department(l: &str) -> Option<&'static str> {
    DEPARTMENTS
        .iter()
        .find(|d| l.eq_ignore_ascii_case(d))
        .copied()
}

fn parse_decimal(tok: &str) -> Option<f64> {
    if tok.contains('.') && tok.chars().all(|c| c.is_ascii_digit() || c == '.') {
        tok.parse().ok()
    } else {
        None
    }
}

/// Fresh Thyme template. Hazards from the real corpus: department headers interleaved,
/// leading-`0` POS artifacts, `N F` tax/food flags, `x` promo markers, `You saved` lines
/// that attach to the *previous* item, two-line items (`GREEN ONIONS` then `2 @ 0.99 EA`),
/// weight items (`3.11 lb @ 1.29/ lb 4.01`), mixed date formats, no tax line.
pub fn parse(text: &str) -> ParsedReceipt {
    let mut r = ParsedReceipt::new();
    let mut dept: Option<String> = None;
    let mut pending: Option<usize> = None; // index of name-only line awaiting qty line

    for raw in text.lines() {
        let l = raw.trim();
        if l.is_empty() {
            continue;
        }
        let up = l.to_ascii_uppercase();

        if up.contains("FRESH THYME MARKET") {
            r.store_name = Some("Fresh Thyme Market".into());
            continue;
        }
        if let Some(d) = is_department(l) {
            dept = Some(d.to_string());
            continue;
        }
        if let Some(dt) = find_datetime(l) {
            if r.occurred_at.is_none() {
                r.occurred_at = Some(dt);
            }
            continue;
        }
        if up.starts_with("INVOICE") {
            r.txn_ref = find_int(l).map(|v| v.to_string());
            continue;
        }
        if up.starts_with("STORE:") {
            let num: String = l[6..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if !num.is_empty() {
                r.store_ref = Some(num);
            }
            continue;
        }
        if up.contains("BALANCE DUE") {
            if let Some(v) = tail_money(l) {
                r.total_cents = Some(v);
                r.total_source = TotalSource::Printed;
            }
            continue;
        }
        if up.starts_with("TOTAL USD") {
            // card-block total near the top; BALANCE DUE is authoritative
            if r.total_cents.is_none() {
                if let Some(v) = tail_money(l) {
                    r.total_cents = Some(v);
                    r.total_source = TotalSource::Printed;
                }
            }
            continue;
        }
        if up.starts_with("TOTAL DISCOUNTS") {
            r.discount_total_cents = tail_money(l);
            continue;
        }
        if up.contains("NUMBER OF ITEMS SOLD") {
            r.item_count = find_int(l);
            continue;
        }
        if up.starts_with("YOU SAVED") {
            if let Some(v) = tail_money(l) {
                if let Some(last) = r.lines.last_mut() {
                    last.savings_cents = Some(v);
                }
            }
            continue;
        }

        // Continuation lines for a pending name-only item:
        //   "2 @ 0.99 EA 1.98 N F"     -> qty x unit = total
        //   "3.11 lb @ 1.29/ lb 4.01 N F" -> weight x price/lb = total
        if let Some(idx) = pending {
            let mut toks: Vec<&str> = l.split_whitespace().collect();
            // continuation lines also end with flags: "2 @ 0.99 EA 1.98 N F"
            crate::chains::pop_flags(&mut toks, &['N', 'F']);
            if let Some(total) = toks.last().and_then(|t| parse_money(t)) {
                if let Some(at) = toks.iter().position(|t| *t == "@") {
                    if at >= 1 && at + 1 < toks.len() {
                        let unit = parse_money(toks[at + 1].trim_end_matches('/'));
                        // "2 @ 0.99 EA 1.98"           -> count x unit
                        let count: Option<i32> = if at == 1 { toks[0].parse().ok() } else { None };
                        // "3.11 lb @ 1.29/ lb 4.01"    -> weight x price/lb
                        let weight: Option<f64> = if at == 2 && toks[1].eq_ignore_ascii_case("lb") {
                            parse_decimal(toks[0])
                        } else {
                            None
                        };
                        if let (Some(unit), Some(q)) = (unit, count) {
                            let line = &mut r.lines[idx];
                            line.qty_count = Some(q);
                            line.unit_price_cents = Some(unit);
                            line.unit_basis = Some("EA".into());
                            line.line_total_cents = total;
                            line.confidence = 0.95;
                            pending = None;
                            continue;
                        }
                        if let (Some(unit), Some(w)) = (unit, weight) {
                            let line = &mut r.lines[idx];
                            line.qty_grams = Some(w * 453.592);
                            line.unit_price_cents = Some(unit);
                            line.unit_basis = Some("LB".into());
                            line.line_total_cents = total;
                            line.confidence = 0.95;
                            pending = None;
                            continue;
                        }
                    }
                }
            }
        }

        // Item lines start with the POS `0 ` artifact.
        if let Some(rest) = l.strip_prefix("0 ") {
            let mut toks: Vec<&str> = rest.split_whitespace().collect();
            let flags = crate::chains::pop_flags(&mut toks, &['N', 'F']);
            let tail = toks.last().copied();
            if let Some(v) = tail.and_then(|t| parse_money(t)) {
                toks.pop();
                // internal promo marker
                let promo = toks.iter().any(|t| *t == "x" || *t == "X" || *t == "*");
                let mut promo_flagged = flags.clone();
                if promo {
                    promo_flagged.push("PROMO".into());
                }
                let name: String = toks
                    .iter()
                    .filter(|t| !matches!(**t, "x" | "X" | "*"))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
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
                    flags: promo_flagged,
                    department: dept.clone(),
                    confidence: 0.95,
                });
                pending = None;
            } else {
                // name-only line; qty/price arrives on the next line
                r.push_line(ItemLine {
                    seq: r.lines.len() + 1,
                    raw_name: toks.join(" "),
                    store_item_code: None,
                    discount_ref: None,
                    qty_count: None,
                    qty_grams: None,
                    unit_basis: None,
                    unit_price_cents: None,
                    line_total_cents: 0,
                    savings_cents: None,
                    flags,
                    department: dept.clone(),
                    confidence: 0.6,
                });
                pending = Some(r.lines.len() - 1);
            }
            continue;
        }
        // everything else: header/survey/card/footer — intentionally ignored
    }

    if let Some(idx) = pending {
        r.issue(
            IssueKind::LowConfidenceLine,
            format!(
                "item '{}' never received a quantity/price line",
                r.lines[idx].raw_name
            ),
        );
    }
    r
}
