//! Fills the app's ledger with synthetic invoices, for looking at the UI with
//! something in it.
//!
//! Run with `cargo run --example seed_sample_data`. It writes to the SAME
//! database the app uses, so quit the app first or expect to hit refresh.
//!
//! The samples are chosen to cover the states the UI has to render, not to be
//! realistic paperwork: one clean invoice per category, one whose amounts do
//! not add up, one that duplicates another's 发票号码 from a different file,
//! and one that is not an invoice at all. Between them they light up every
//! badge, every warning, and the empty-vs-populated branch of every pane.
//!
//! Everything goes through the real pipeline - `extract` → `validate` →
//! `classify` → `db::save` - so a run of this is also an end-to-end check that
//! those four fit together outside the test harness.

use invoice_desk_lib::{classify, db, extract};

/// A 数电票 XML, in the shape [`extract::einvoice_xml`] parses.
fn invoice_xml(
    number: &str,
    date: &str,
    seller: &str,
    item: &str,
    net: &str,
    tax: &str,
    total: &str,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<EInvoice>
  <EInvoiceData>
    <SellerInformation>
      <SellerName>{seller}</SellerName>
      <SellerTaxID>91330106MA2GX8QP1A</SellerTaxID>
    </SellerInformation>
    <BuyerInformation>
      <BuyerName>猫芬奇工作室</BuyerName>
      <BuyerTaxID>91310115MA1K3XYZ2B</BuyerTaxID>
    </BuyerInformation>
    <BasicInformation>
      <InvoiceType>电子发票(普通发票)</InvoiceType>
      <InvoiceNumber>{number}</InvoiceNumber>
      <IssueTime>{date}</IssueTime>
      <TotalAmWithoutTax>{net}</TotalAmWithoutTax>
      <TotalTaxAm>{tax}</TotalTaxAm>
      <TotalTax-includedAmount>{total}</TotalTax-includedAmount>
    </BasicInformation>
    <IssuItemInformation>
      <ItemName>{item}</ItemName>
      <Quantity>1</Quantity>
      <UnPrice>{net}</UnPrice>
      <Amount>{net}</Amount>
      <TaxRateValue>0.06</TaxRateValue>
      <ComTaxAm>{tax}</ComTaxAm>
    </IssuItemInformation>
  </EInvoiceData>
</EInvoice>"#
    )
}

fn main() -> Result<(), String> {
    let dir = std::env::temp_dir().join("invoicedesk-samples");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // (文件名, 内容) - the trailing comment on each says which UI state it is
    // there to produce.
    let samples: Vec<(String, String)> = vec![
        // Clean, one per category, so the 分类汇总 has something to group.
        (
            "住宿-云栖酒店.xml".into(),
            invoice_xml(
                "24312000000010000001",
                "2024-03-01",
                "杭州云栖酒店管理有限公司",
                "*住宿服务*住宿费",
                "1000.00",
                "60.00",
                "1060.00",
            ),
        ),
        (
            "餐饮-某某餐厅.xml".into(),
            invoice_xml(
                "24312000000010000002",
                "2024-03-03",
                "上海某某餐饮管理有限公司",
                "*餐饮服务*餐费",
                "252.83",
                "15.17",
                "268.00",
            ),
        ),
        (
            "交通-滴滴.xml".into(),
            invoice_xml(
                "24312000000010000003",
                "2024-03-05",
                "滴滴出行科技有限公司",
                "*运输服务*客运服务费",
                "41.05",
                "2.45",
                "43.50",
            ),
        ),
        (
            "软件-某某科技.xml".into(),
            invoice_xml(
                "24312000000010000004",
                "2024-03-07",
                "杭州某某科技有限公司",
                "*信息技术服务*技术服务费",
                "5000.00",
                "300.00",
                "5300.00",
            ),
        ),
        (
            "办公-文具.xml".into(),
            invoice_xml(
                "24312000000010000005",
                "2024-03-11",
                "上海某某文化用品有限公司",
                "*办公用品*A4复印纸",
                "265.49",
                "34.51",
                "300.00",
            ),
        ),
        // 价税合计 does not equal 金额 + 税额: lights up the red 校验错误 and
        // puts the row in 待复核.
        (
            "问题-金额对不上.xml".into(),
            invoice_xml(
                "24312000000010000006",
                "2024-03-13",
                "某某会议服务有限公司",
                "*会议服务*会务费",
                "2000.00",
                "120.00",
                "2100.00",
            ),
        ),
        // The same 发票号码 as the first, from a different file: 疑似重复.
        (
            "重复-云栖酒店(另一份).xml".into(),
            invoice_xml(
                "24312000000010000001",
                "2024-03-01",
                "杭州云栖酒店管理有限公司",
                "*住宿服务*住宿费",
                "1000.00",
                "60.00",
                "1060.00",
            ) + "\n<!-- 同一张发票的另一个副本，字节不同 -->",
        ),
        // No rule matches this one: 未分类, and the place the AI suggestion
        // button earns its keep.
        (
            "未分类-某某贸易.xml".into(),
            invoice_xml(
                "24312000000010000007",
                "2024-03-17",
                "某某贸易有限公司",
                "*未列明商品*杂项",
                "800.00",
                "104.00",
                "904.00",
            ),
        ),
        // Not an invoice at all: 识别失败, stored anyway so it is not lost.
        ("读不出来.txt".into(), "这是一份合同，不是发票。".into()),
    ];

    for (name, body) in &samples {
        std::fs::write(dir.join(name), body).map_err(|e| e.to_string())?;
    }

    // The same database the app opens. Kept in step with `lib.rs`'s
    // `database_path` by hand - there is no AppHandle out here to ask.
    let db_path = dirs_app_data()?.join("invoicedesk.db");
    println!("账本：{}", db_path.display());
    let connection = db::open(&db_path)?;
    db::seed_rules_if_empty(&connection)?;
    let rules = db::load_rules(&connection)?;

    let (mut imported, mut duplicates) = (0, 0);
    for (name, _) in &samples {
        let path = dir.join(name);
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;

        let mut outcome = extract::extract(&bytes, &path);
        if let classify::Outcome::Matched { category, rule } =
            classify::classify(&outcome.invoice, &rules)
        {
            outcome.invoice.category = Some(category);
            outcome.invoice.category_rule = Some(rule);
        }

        match db::save(&connection, &outcome.invoice)? {
            db::SaveOutcome::AlreadyImported { .. } => duplicates += 1,
            _ => imported += 1,
        }

        let status = if extract::is_complete(&outcome.invoice) {
            "已导入"
        } else if outcome.invoice.number.is_present() {
            "待复核"
        } else {
            "识别失败"
        };
        println!(
            "  {status}  {name}  {}",
            outcome
                .invoice
                .category
                .as_deref()
                .unwrap_or(classify::UNCATEGORISED)
        );
    }

    println!("\n新增 {imported} 张，跳过重复文件 {duplicates} 个。");
    println!("样例文件在：{}", dir.display());
    Ok(())
}

/// The platform app-data directory, resolved without Tauri's `AppHandle`.
fn dirs_app_data() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "找不到 HOME".to_string())?;
    let base = if cfg!(target_os = "macos") {
        std::path::PathBuf::from(home).join("Library/Application Support")
    } else if cfg!(target_os = "windows") {
        std::path::PathBuf::from(std::env::var("APPDATA").map_err(|_| "找不到 APPDATA")?)
    } else {
        std::path::PathBuf::from(home).join(".local/share")
    };
    Ok(base.join("com.chengaoshen.invoicedesk"))
}
