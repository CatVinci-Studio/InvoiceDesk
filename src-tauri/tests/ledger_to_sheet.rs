//! End to end, over the real modules: bytes → ledger → reimbursement sheet.
//!
//! The unit tests cover each stage against hand-built inputs. What none of
//! them can prove is that the stages still line up once real data has been
//! through a SQLite round trip - which is where a reimbursement tool would
//! actually go wrong, because that is where an amount stops being an in-memory
//! `Money` and becomes a row somebody has to trust.
//!
//! So this walks the whole path with no mocks: an invoice XML goes in as
//! bytes, gets extracted, validated, classified, stored, read back, and
//! written to an .xlsx - and the workbook is then reopened with calamine, the
//! way Excel would open it, and its numbers compared against the 分 the ledger
//! holds.

use invoice_desk_lib::{classify, db, extract, model::Money, report};

/// A 数电票 XML in the shape the tax platform issues.
fn invoice_xml(
    number: &str,
    seller: &str,
    item: &str,
    net: &str,
    tax: &str,
    total: &str,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<EInvoice><EInvoiceData>
  <SellerInformation><SellerName>{seller}</SellerName></SellerInformation>
  <BuyerInformation><BuyerName>猫芬奇工作室</BuyerName></BuyerInformation>
  <BasicInformation>
    <InvoiceType>电子发票(普通发票)</InvoiceType>
    <InvoiceNumber>{number}</InvoiceNumber>
    <IssueTime>2024-03-01</IssueTime>
    <TotalAmWithoutTax>{net}</TotalAmWithoutTax>
    <TotalTaxAm>{tax}</TotalTaxAm>
    <TotalTax-includedAmount>{total}</TotalTax-includedAmount>
  </BasicInformation>
  <IssuItemInformation><ItemName>{item}</ItemName><Amount>{net}</Amount></IssuItemInformation>
</EInvoiceData></EInvoice>"#
    )
}

/// Imports one file the way `commands::ingest` does, minus the Tauri parts.
fn import(
    connection: &rusqlite::Connection,
    name: &str,
    xml: &str,
    rules: &[classify::Rule],
) -> i64 {
    let path = std::path::PathBuf::from(name);
    let mut outcome = extract::extract(xml.as_bytes(), &path);
    if let classify::Outcome::Matched { category, rule } =
        classify::classify(&outcome.invoice, rules)
    {
        outcome.invoice.category = Some(category);
        outcome.invoice.category_rule = Some(rule);
    }
    db::save(connection, &outcome.invoice).expect("入库").id()
}

/// Every numeric cell of a sheet, keyed by (row, column).
fn numbers(path: &std::path::Path, sheet: usize) -> std::collections::HashMap<(usize, usize), f64> {
    use calamine::{Data, Reader};
    let mut book: calamine::Xlsx<_> = calamine::open_workbook(path).expect("打开导出的工作簿");
    let name = book.sheet_names()[sheet].clone();
    let range = book.worksheet_range(&name).expect("读取工作表");
    range
        .cells()
        .filter_map(|(row, col, cell)| match cell {
            Data::Float(v) => Some(((row, col), *v)),
            Data::Int(v) => Some(((row, col), *v as f64)),
            _ => None,
        })
        .collect()
}

#[test]
fn an_invoice_survives_the_whole_path_from_bytes_to_spreadsheet() {
    let connection = db::open_in_memory().expect("账本");
    db::seed_rules_if_empty(&connection).expect("内置规则");
    let rules = db::load_rules(&connection).expect("读取规则");

    let ids = [
        import(
            &connection,
            "住宿.xml",
            &invoice_xml(
                "24312000000010000001",
                "杭州云栖酒店管理有限公司",
                "*住宿服务*住宿费",
                "1000.00",
                "60.00",
                "1060.00",
            ),
            &rules,
        ),
        // 0.07 and 43.50 are the amounts a float pipeline mangles: one is not
        // representable in binary, the other loses its trailing zero.
        import(
            &connection,
            "交通.xml",
            &invoice_xml(
                "24312000000010000002",
                "滴滴出行科技有限公司",
                "*运输服务*客运服务费",
                "41.05",
                "2.45",
                "43.50",
            ),
            &rules,
        ),
        import(
            &connection,
            "零头.xml",
            &invoice_xml(
                "24312000000010000003",
                "某某便利店",
                "*餐饮服务*餐费",
                "0.07",
                "0.00",
                "0.07",
            ),
            &rules,
        ),
    ];

    // The rules must have fired on the way in - an uncategorised sheet is a
    // sheet somebody has to sort by hand.
    let rows = db::list(&connection, &db::Filter::default()).expect("列表");
    assert_eq!(rows.len(), 3);
    let categories: Vec<&str> = rows.iter().map(|r| r.category.as_str()).collect();
    for expected in ["住宿", "市内交通", "餐饮"] {
        assert!(categories.contains(&expected), "缺少类别 {expected}");
    }

    // Nothing should be flagged: every field came from XML and the amounts
    // add up. A spurious issue here means `validate` and the extractors have
    // drifted apart, which is exactly what shipped broken once before.
    assert!(
        rows.iter().all(|r| r.issue_count == 0),
        "干净的发票不该有校验问题：{:?}",
        rows.iter()
            .map(|r| (&r.number, r.issue_count))
            .collect::<Vec<_>>()
    );

    let report_id = db::save_report(
        &connection,
        &db::Report {
            id: None,
            title: "2024年3月报销".into(),
            applicant: "沈".into(),
            department: "研发".into(),
            note: String::new(),
            created_at: String::new(),
            invoice_count: 0,
            total_cents: 0,
        },
    )
    .expect("新建报销单");
    db::set_report_invoices(&connection, report_id, &ids).expect("加入发票");

    let invoices = db::report_invoices(&connection, report_id).expect("读取报销单");
    assert_eq!(invoices.len(), 3);
    assert_eq!(
        invoices.iter().map(|i| i.id.unwrap()).collect::<Vec<_>>(),
        ids.to_vec(),
        "报销单必须保留加入的顺序"
    );

    let summary = report::Summary::of(&invoices);
    // 1060.00 + 43.50 + 0.07
    assert_eq!(summary.total, Money(110_357));
    assert_eq!(summary.amount_excl_tax + summary.tax, summary.total);
    assert_eq!(
        report::chinese_capital(summary.total),
        "壹仟壹佰零叁圆伍角柒分"
    );

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("报销单.xlsx");
    report::workbook(
        &invoices,
        &report::ReportMeta {
            title: "2024年3月报销".into(),
            applicant: "沈".into(),
            department: "研发".into(),
            note: String::new(),
            date: "2024-04-01".into(),
        },
        &out,
    )
    .expect("导出");

    // --- and now read it back the way Excel would ---
    let detail = numbers(&out, 0);
    // Detail rows start at row 4 (title, header block, blank, column headers);
    // 价税合计 is column 8. Compared in 分, so a float that drifted by a
    // hundredth fails rather than rounding into agreement.
    let cents = |value: f64| (value * 100.0).round() as i64;
    assert_eq!(cents(detail[&(4, 8)]), 106_000, "住宿那行的价税合计");
    assert_eq!(cents(detail[&(5, 8)]), 4_350, "交通那行——43.50 的尾零");
    assert_eq!(
        cents(detail[&(6, 8)]),
        7,
        "0.07——二进制浮点表示不了的那个数"
    );

    // The 合计 row must equal the ledger, to the fen.
    assert_eq!(cents(detail[&(7, 8)]), summary.total.cents(), "合计行");

    // And the summary sheet has to agree with the detail sheet, or the two
    // tabs of one workbook contradict each other.
    let totals = numbers(&out, 1);
    let summed: i64 = (1..=3).map(|row| cents(totals[&(row, 4)])).sum();
    assert_eq!(summed, summary.total.cents(), "分类汇总要等于明细合计");
}

/// A workbook with no invoices still has to be a valid, openable file - an
/// export that silently produces a corrupt sheet is worse than one that fails.
#[test]
fn an_empty_report_still_exports_a_readable_workbook() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("空.xlsx");
    report::workbook(&[], &report::ReportMeta::default(), &out).expect("导出");

    use calamine::Reader;
    let book: calamine::Xlsx<_> = calamine::open_workbook(&out).expect("空表也要能打开");
    assert_eq!(book.sheet_names().len(), 2, "明细表 + 汇总表");
}
