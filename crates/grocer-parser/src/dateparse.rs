//! Normalize the date/time formats seen on real receipts to `YYYY-MM-DDTHH:MM[:SS]`.
//! Handles `01/27/2025 19:34:01` (Fresh Thyme, Costco) and `02-14-25 17:22` (Trader Joe's).

fn norm_time(t: &str) -> Option<String> {
    let core: String = t
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ':')
        .collect();
    let parts: Vec<&str> = core.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    if !parts
        .iter()
        .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    Some(core)
}

/// Scan the line's tokens for a recognizable date (optionally followed by a time).
pub fn find_datetime(line: &str) -> Option<String> {
    let toks: Vec<&str> = line.split_whitespace().collect();
    for (i, t) in toks.iter().enumerate() {
        let all_digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
        let b = t.as_bytes();
        // MM/DD/YYYY
        if b.len() == 10
            && b[2] == b'/'
            && b[5] == b'/'
            && all_digits(&t[0..2])
            && all_digits(&t[3..5])
            && all_digits(&t[6..10])
        {
            let m: u32 = t[0..2].parse().ok()?;
            let d: u32 = t[3..5].parse().ok()?;
            let y: u32 = t[6..10].parse().ok()?;
            if m == 0 || m > 12 || d == 0 || d > 31 {
                continue;
            }
            let time = toks.get(i + 1).and_then(|n| norm_time(n));
            return Some(match time {
                Some(tm) => format!("{y:04}-{m:02}-{d:02}T{tm}"),
                None => format!("{y:04}-{m:02}-{d:02}"),
            });
        }
        // MM-DD-YY
        if b.len() == 8
            && b[2] == b'-'
            && b[5] == b'-'
            && all_digits(&t[0..2])
            && all_digits(&t[3..5])
            && all_digits(&t[6..8])
        {
            let m: u32 = t[0..2].parse().ok()?;
            let d: u32 = t[3..5].parse().ok()?;
            let y: u32 = 2000 + t[6..8].parse::<u32>().ok()?;
            if m == 0 || m > 12 || d == 0 || d > 31 {
                continue;
            }
            let time = toks.get(i + 1).and_then(|n| norm_time(n));
            return Some(match time {
                Some(tm) => format!("{y:04}-{m:02}-{d:02}T{tm}"),
                None => format!("{y:04}-{m:02}-{d:02}"),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(
            find_datetime("01/27/2025 19:34:01"),
            Some("2025-01-27T19:34:01".into())
        );
        assert_eq!(
            find_datetime("02/21/2026 17:47 652 4 522 24"),
            Some("2026-02-21T17:47".into())
        );
        assert_eq!(
            find_datetime("DATE 02-14-25  17:22"),
            Some("2025-02-14T17:22".into())
        );
        // hazards that must not match
        assert_eq!(find_datetime("EXP. DATE: 00/2000"), None);
        assert_eq!(find_datetime("TICKET#:6250 27JAN2025 19:34:03"), None);
        assert_eq!(find_datetime("CASHIER NAME: SC012"), None);
    }
}
