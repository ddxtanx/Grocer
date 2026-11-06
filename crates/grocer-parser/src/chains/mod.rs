pub mod costco;
pub mod fresh_thyme;
pub mod trader_joes;

use crate::model::{IssueKind, ParsedReceipt};

/// Strip PII from raw text before it ever leaves the device or gets stored:
/// membership numbers (Costco `4Y Member 111894750330`) — card PANs are already
/// X-masked at the register. Returns the masked text; notes issues on the receipt.
pub fn mask_pii(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.to_ascii_lowercase().contains("member") {
                let mut out = String::new();
                let mut chars = line.chars().peekable();
                while let Some(c) = chars.next() {
                    if c.is_ascii_digit() {
                        let mut run = String::new();
                        run.push(c);
                        while let Some(&d) = chars.peek() {
                            if d.is_ascii_digit() {
                                run.push(d);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                        // membership numbers: 8+ digits (leave small integers alone)
                        if run.len() >= 8 {
                            out.push_str("[REDACTED-MEMBER]");
                        } else {
                            out.push_str(&run);
                        }
                    } else {
                        out.push(c);
                    }
                }
                out
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Trailing single-char flag tokens used across chains (`N F`, `A`, ...).
pub fn pop_flags(toks: &mut Vec<&str>, allowed: &[char]) -> Vec<String> {
    let mut flags = Vec::new();
    while let Some(&last) = toks.last() {
        if last.len() == 1 && allowed.contains(&last.chars().next().unwrap()) {
            flags.push(last.to_ascii_uppercase());
            toks.pop();
        } else {
            break;
        }
    }
    flags.reverse();
    flags
}

pub fn note_pii(r: &mut ParsedReceipt) {
    r.issue(
        IssueKind::PiiMasked,
        "membership number redacted from raw text",
    );
}
