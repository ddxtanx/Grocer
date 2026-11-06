/// Parse a money token into integer cents.
/// Accepted: "38.92", "$38.92", "1,234.56", "5.00-" (trailing-dash negative, Costco style),
/// "-0.90" (leading-dash negative). Requiring exactly 2 decimal places keeps phone numbers,
/// integers and rates from masquerading as money.
pub fn parse_money(tok: &str) -> Option<i64> {
    let mut t = tok.trim();
    if t.is_empty() {
        return None;
    }
    let mut neg = false;
    if let Some(rest) = t.strip_suffix('-') {
        neg = true;
        t = rest;
    }
    if let Some(rest) = t.strip_prefix('-') {
        neg = true;
        t = rest;
    }
    if let Some(rest) = t.strip_prefix('$') {
        t = rest;
    }
    if t.is_empty() {
        return None;
    }
    let clean: String = t.chars().filter(|c| *c != ',').collect();
    if !clean.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let dot = clean.find('.')?;
    let whole = &clean[..dot];
    let frac = &clean[dot + 1..];
    if frac.len() != 2 {
        return None;
    }
    let whole_v: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let frac_v: i64 = frac.parse().ok()?;
    let v = whole_v * 100 + frac_v;
    Some(if neg { -v } else { v })
}

/// Money value of the last token on a line, if it is money.
pub fn tail_money(line: &str) -> Option<i64> {
    parse_money(line.split_whitespace().last()?)
}

/// First integer found in a line (for counts).
pub fn find_int(line: &str) -> Option<i32> {
    for tok in line.split_whitespace() {
        let core: String = tok.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !core.is_empty() && tok.chars().all(|c| c.is_ascii_digit()) {
            return core.parse().ok();
        }
    }
    // fall back: any run of digits
    let run: String = line
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    run.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_forms() {
        assert_eq!(parse_money("38.92"), Some(3892));
        assert_eq!(parse_money("$46.85"), Some(4685));
        assert_eq!(parse_money("5.00-"), Some(-500));
        assert_eq!(parse_money("-0.90"), Some(-90));
        assert_eq!(parse_money("1,234.56"), Some(123456));
        assert_eq!(parse_money("9.025%"), None);
        assert_eq!(parse_money("612-339-2110"), None);
        assert_eq!(parse_money("126104"), None);
        assert_eq!(parse_money("101"), None);
        assert_eq!(parse_money("$"), None);
        assert_eq!(tail_money("E 49000 KS SOY VNILA 14.99"), Some(1499));
        assert_eq!(tail_money("MEATLESS KOREAN BULGOGI 4.99"), Some(499));
    }
}
