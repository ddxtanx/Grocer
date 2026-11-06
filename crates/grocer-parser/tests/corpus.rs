use grocer_parser::*;

fn find<'a>(r: &'a ParsedReceipt, name: &str) -> &'a ItemLine {
    r.lines.iter().find(|l| l.raw_name == name).expect(name)
}

fn has_issue(r: &ParsedReceipt, kind: IssueKind) -> bool {
    r.issues.iter().any(|i| i.kind == kind)
}

#[test]
fn costco_redacted_total_recovers_from_payment_block() {
    let text = include_str!("fixtures/costco_652_redacted.txt");
    let r = parse(text);
    assert_eq!(r.chain, Chain::Costco);
    assert_eq!(r.store_ref.as_deref(), Some("652"));
    assert_eq!(r.occurred_at.as_deref(), Some("2026-02-21T17:47"));
    assert_eq!(r.txn_ref.as_deref(), Some("605200004531"));

    // 17 item lines + 1 coupon line
    assert_eq!(
        r.lines.len(),
        18,
        "lines: {:?}",
        r.lines.iter().map(|l| &l.raw_name).collect::<Vec<_>>()
    );
    assert_eq!(r.subtotal_cents, Some(17963));
    assert_eq!(r.tax_cents, Some(332));
    assert_eq!(r.tax_rate_milli_pct, Some(9025));

    // The printed TOTAL was blacked out in every sample photo. Recovery path:
    // payment block -> 182.95, then verified against subtotal+tax by validate().
    assert_eq!(r.total_cents, Some(18295));
    assert_eq!(r.total_source, TotalSource::InferredFromPayment);
    assert!(has_issue(&r, IssueKind::TotalMissing));

    // Coupon line references the item it discounts.
    let coupon = r
        .lines
        .iter()
        .find(|l| l.discount_ref.is_some())
        .expect("coupon");
    assert_eq!(coupon.discount_ref.as_deref(), Some("1961075"));
    assert_eq!(coupon.line_total_cents, -500);
    assert_eq!(r.discount_total_cents, Some(-500));

    // Flags and codes survive.
    let cat = find(&r, "KS CAT PATE");
    assert_eq!(cat.store_item_code.as_deref(), Some("1647381"));
    assert!(cat.flags.contains(&"A".to_string()));

    // Membership number is PII: masked in stored raw text, noted in issues.
    assert!(!r.raw_text.contains("111894750330"));
    assert!(r.raw_text.contains("[REDACTED-MEMBER]"));
    assert!(has_issue(&r, IssueKind::PiiMasked));

    // No arithmetic issues should remain.
    let hard: Vec<_> = r
        .issues
        .iter()
        .filter(|i| {
            matches!(
                i.kind,
                IssueKind::SumMismatch | IssueKind::LineMathMismatch | IssueKind::TaxMismatch
            )
        })
        .collect();
    assert!(hard.is_empty(), "unexpected hard issues: {hard:?}");
}

#[test]
fn trader_joes_repeated_lines_merge_and_checksums_close() {
    let text = include_str!("fixtures/traderjoes_725.txt");
    let r = parse(text);
    assert_eq!(r.chain, Chain::TraderJoes);
    assert_eq!(r.store_ref.as_deref(), Some("0725"));
    assert_eq!(r.occurred_at.as_deref(), Some("2025-02-14T17:22"));
    assert_eq!(r.txn_ref.as_deref(), Some("247673"));
    assert_eq!(r.item_count, Some(12));

    // 6 distinct lines after merging repeats (10 raw lines: bulgogi x3, noodles x2, veg rice x2)
    assert_eq!(r.lines.len(), 6);
    assert_eq!(find(&r, "MEATLESS KOREAN BULGOGI").qty_count, Some(3));
    assert_eq!(find(&r, "MEATLESS KOREAN BULGOGI").line_total_cents, 1497);
    assert_eq!(find(&r, "FLATBREAD INDIAN STYLE G").line_total_cents, 399);

    // jelly beans: sub-line "3 @ $2.49" attaches qty+unit
    let jb = find(&r, "T JELLY BEANS SOUR");
    assert_eq!(jb.qty_count, Some(3));
    assert_eq!(jb.unit_price_cents, Some(249));
    assert_eq!(jb.line_total_cents, 747);

    assert_eq!(r.tax_cents, Some(67));
    assert_eq!(r.tax_rate_milli_pct, Some(9025));
    assert_eq!(r.total_cents, Some(4685));
    assert_eq!(r.subtotal_cents, Some(4618)); // synthesized: TJ prints no subtotal

    let hard: Vec<_> = r
        .issues
        .iter()
        .filter(|i| {
            matches!(
                i.kind,
                IssueKind::SumMismatch | IssueKind::LineMathMismatch | IssueKind::ItemCountMismatch
            )
        })
        .collect();
    assert!(hard.is_empty(), "unexpected hard issues: {hard:?}");
}

#[test]
fn fresh_thyme_departs_two_line_items_and_savings() {
    let text = include_str!("fixtures/freshthyme_905.txt");
    let r = parse(text);
    assert_eq!(r.chain, Chain::FreshThyme);
    assert_eq!(r.store_ref.as_deref(), Some("00905"));
    assert_eq!(r.occurred_at.as_deref(), Some("2025-01-27T19:34:01"));
    assert_eq!(r.txn_ref.as_deref(), Some("126104"));

    assert_eq!(r.lines.len(), 9);
    assert_eq!(r.total_cents, Some(3892));
    assert_eq!(r.subtotal_cents, Some(3892)); // no tax line on this all-food basket
    assert_eq!(r.tax_cents, None);
    assert_eq!(r.discount_total_cents, Some(-90));

    // departments captured
    assert_eq!(
        find(&r, "CHEESE ALT AMERICN").department.as_deref(),
        Some("DAIRY")
    );
    assert_eq!(
        find(&r, "EDAMAME DRY RST").department.as_deref(),
        Some("GROCERY")
    );
    assert_eq!(find(&r, "CILANTRO").department.as_deref(), Some("PRODUCE"));

    // promo marker
    assert!(find(&r, "GF CHOC CHNK")
        .flags
        .contains(&"PROMO".to_string()));
    // "You saved" attaches to the preceding item
    assert_eq!(find(&r, "GF CHOC CHNK").savings_cents, Some(50));
    assert_eq!(find(&r, "EDAMAME DRY RST").savings_cents, Some(40));

    // two-line item: name, then "2 @ 0.99 EA 1.98"
    let go = find(&r, "GREEN ONIONS");
    assert_eq!(go.qty_count, Some(2));
    assert_eq!(go.unit_price_cents, Some(99));
    assert_eq!(go.line_total_cents, 198);

    // weight item: "3.11 lb @ 1.29/ lb 4.01"
    let rc = find(&r, "RED CABBAGE");
    assert_eq!(rc.unit_basis.as_deref(), Some("LB"));
    assert_eq!(rc.unit_price_cents, Some(129));
    assert_eq!(rc.line_total_cents, 401);
    assert!(rc.qty_grams.unwrap() > 1400.0 && rc.qty_grams.unwrap() < 1420.0);

    assert!(!has_issue(&r, IssueKind::ItemCountMismatch)); // closes: 7 singles + 2 onions + 1 cabbage = 10

    let hard: Vec<_> = r
        .issues
        .iter()
        .filter(|i| matches!(i.kind, IssueKind::SumMismatch | IssueKind::LineMathMismatch))
        .collect();
    assert!(hard.is_empty(), "unexpected hard issues: {hard:?}");
}

#[test]
fn server_side_verify_catches_tampering() {
    let text = include_str!("fixtures/traderjoes_725.txt");
    let mut r = parse(text);
    // device "misreads" a price: line no longer matches claimed totals
    r.lines.get_mut(0).unwrap().line_total_cents = 300;
    let issues = verify(&r);
    assert!(issues.iter().any(|i| i.kind == IssueKind::SumMismatch));
}

#[test]
fn unknown_chain_yields_no_lines() {
    let r = parse("SOME CORNER STORE\n1 thing 2.00\nTOTAL 2.00");
    assert_eq!(r.chain, Chain::Unknown);
    assert!(has_issue(&r, IssueKind::ParsedNothing));
}

#[test]
fn detects_costco_from_mangled_header_ocr() {
    // Real OCR tail captured from a user scan: header/logo read as garbage,
    // footer trio intact. Detection must not depend on the header word.
    let ocr = "— ~ TOTAL TAX 3.32\n\
               — TOTAL NUMBER OF ITEMS SOLD = 17 .\n\
               x INSTANT SAVINGS $ 5.00 -\n\
               - \\YIRANPIYIE 17:47 652 4 522 24\n\
               QT |\n\
               OP#: 24 Name: LUCAS J.\n\
               - Thank You! i\n\
               Please Come Again E\n\
               Whse:652 Trm:4 Trn:522 OP:24 v\n\
               Items Sold: 17\n\
               4Y 02/21/2026 17:47 \u{a9}";
    assert_eq!(grocer_parser::detect(ocr), grocer_parser::Chain::Costco);
}
