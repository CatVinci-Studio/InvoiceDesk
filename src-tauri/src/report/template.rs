//! Filling in a company's own .xlsx 报销单 template.
//!
//! Most Chinese companies mandate a form: a specific workbook with the
//! company's letterhead, its own column order, and a signature block at the
//! bottom. Handing finance a differently-shaped sheet gets it sent back, so
//! "export to Excel" is only half a feature - the other half is exporting
//! into *their* Excel.
//!
//! ## Why this edits the XML instead of rebuilding the workbook
//!
//! `rust_xlsxwriter` writes new workbooks; it cannot open one. calamine can
//! read one, but only its values - not its fonts, borders, merged cells,
//! column widths, or letterhead image. Round-tripping through either would
//! hand back a workbook that has the company's *content* and none of its
//! *form*, which defeats the purpose.
//!
//! An .xlsx is a zip of XML. Substituting placeholder text inside the sheet
//! XML and re-zipping leaves every byte of styling untouched, because the
//! styling lives in other parts of the archive that are never opened. That is
//! why this module does its own XML surgery rather than using a library.
//!
//! ## The template language
//!
//! Type these into any cell of the template:
//!
//! | 占位符 | 含义 |
//! |--------|------|
//! | `{{标题}}` `{{申请人}}` `{{部门}}` `{{日期}}` `{{说明}}` | 表头信息 |
//! | `{{张数}}` `{{不含税}}` `{{税额}}` `{{合计}}` `{{合计大写}}` `{{可抵扣税额}}` | 汇总数字 |
//! | `{{明细.序号}}` `{{明细.日期}}` `{{明细.号码}}` `{{明细.票种}}` `{{明细.销售方}}` `{{明细.类别}}` `{{明细.不含税}}` `{{明细.税额}}` `{{明细.合计}}` `{{明细.问题}}` | 明细列 |
//!
//! Any row containing a `{{明细.*}}` placeholder is the **detail template
//! row**: it is cloned once per invoice, with the company's own formatting on
//! every copy, and the rows below it shift down to make room.

use super::{chinese_capital, ReportMeta, Summary};
use crate::classify::UNCATEGORISED;
use crate::model::{Invoice, Money};
use regex::Regex;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::LazyLock;

/// Every placeholder the template understands, for the reference list the
/// settings pane shows next to the template picker.
pub fn placeholders() -> Vec<(&'static str, &'static str)> {
    vec![
        ("{{标题}}", "报销单标题"),
        ("{{申请人}}", "申请人姓名"),
        ("{{部门}}", "部门"),
        ("{{日期}}", "制表日期"),
        ("{{说明}}", "备注说明"),
        ("{{张数}}", "发票张数"),
        ("{{不含税}}", "合计金额（不含税）"),
        ("{{税额}}", "合计税额"),
        ("{{合计}}", "价税合计"),
        ("{{合计大写}}", "价税合计（中文大写）"),
        ("{{可抵扣税额}}", "其中可抵扣进项税额"),
        ("{{明细.序号}}", "明细行：序号"),
        ("{{明细.日期}}", "明细行：开票日期"),
        ("{{明细.号码}}", "明细行：发票号码"),
        ("{{明细.票种}}", "明细行：票种"),
        ("{{明细.销售方}}", "明细行：销售方名称"),
        ("{{明细.类别}}", "明细行：费用类别"),
        ("{{明细.不含税}}", "明细行：金额（不含税）"),
        ("{{明细.税额}}", "明细行：税额"),
        ("{{明细.合计}}", "明细行：价税合计"),
        ("{{明细.问题}}", "明细行：校验问题"),
    ]
}

/// A value bound to a placeholder.
///
/// Three variants rather than two, and the third is the point: **money never
/// becomes a float on this path.**
///
/// A spreadsheet's numeric cell holds the digits `<v>1060.00</v>` in XML;
/// the IEEE double only exists once Excel parses that text. `rust_xlsxwriter`
/// forces a round-trip through `f64` because its API takes one, but this
/// exporter writes the cell XML itself - so it can put
/// [`Money::to_decimal_string`] straight in and skip the float entirely. The
/// digits on disk are then computed from the integer 分 in the ledger by
/// integer division, with nothing in between.
///
/// [`Value::Number`] remains for counts (张数, 序号), which are integers
/// that were never money.
///
/// Text vs number is not cosmetic either: an amount written as a string
/// lands in Excel left-aligned and invisible to `SUM`, which is precisely
/// the bug that makes an accountant stop trusting the tool.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Text(String),
    Number(f64),
    Money(Money),
}

fn scalar_bindings(summary: &Summary, meta: &ReportMeta) -> HashMap<String, Value> {
    HashMap::from([
        ("{{标题}}".into(), Value::Text(meta.title.clone())),
        ("{{申请人}}".into(), Value::Text(meta.applicant.clone())),
        ("{{部门}}".into(), Value::Text(meta.department.clone())),
        ("{{日期}}".into(), Value::Text(meta.date.clone())),
        ("{{说明}}".into(), Value::Text(meta.note.clone())),
        ("{{张数}}".into(), Value::Number(summary.count as f64)),
        ("{{不含税}}".into(), money(summary.amount_excl_tax)),
        ("{{税额}}".into(), money(summary.tax)),
        ("{{合计}}".into(), money(summary.total)),
        (
            "{{合计大写}}".into(),
            Value::Text(chinese_capital(summary.total)),
        ),
        ("{{可抵扣税额}}".into(), money(summary.deductible_tax)),
    ])
}

fn detail_bindings(invoice: &Invoice, index: usize) -> HashMap<String, Value> {
    let text = |s: Option<&String>| Value::Text(s.cloned().unwrap_or_default());
    HashMap::from([
        ("{{明细.序号}}".into(), Value::Number((index + 1) as f64)),
        ("{{明细.日期}}".into(), text(invoice.issued_on.as_ref())),
        ("{{明细.号码}}".into(), text(invoice.number.as_ref())),
        (
            "{{明细.票种}}".into(),
            Value::Text(invoice.kind.map(|k| k.label()).unwrap_or("").to_string()),
        ),
        ("{{明细.销售方}}".into(), text(invoice.seller_name.as_ref())),
        (
            "{{明细.类别}}".into(),
            Value::Text(
                invoice
                    .category
                    .clone()
                    .filter(|c| !c.is_empty())
                    .unwrap_or_else(|| UNCATEGORISED.to_string()),
            ),
        ),
        (
            "{{明细.不含税}}".into(),
            money(invoice.amount_excl_tax.value.unwrap_or(Money::ZERO)),
        ),
        (
            "{{明细.税额}}".into(),
            money(invoice.tax.value.unwrap_or(Money::ZERO)),
        ),
        (
            "{{明细.合计}}".into(),
            money(invoice.total.value.unwrap_or(Money::ZERO)),
        ),
        (
            "{{明细.问题}}".into(),
            Value::Text(
                invoice
                    .issues
                    .iter()
                    .map(|i| i.message())
                    .collect::<Vec<_>>()
                    .join("；"),
            ),
        ),
    ])
}

fn money(amount: Money) -> Value {
    Value::Money(amount)
}

// ---------------------------------------------------------------------------
// Sheet XML surgery
// ---------------------------------------------------------------------------

static ROW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<row\b[^>]*/>|<row\b[^>]*>.*?</row>").unwrap());
static CELL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<c\b[^>]*/>|<c\b[^>]*>.*?</c>").unwrap());
static ATTR_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\br="([A-Z]+\d+)""#).unwrap());
static ATTR_ROW_NUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\br="(\d+)""#).unwrap());
static ATTR_STYLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bs="(\d+)""#).unwrap());
static ATTR_TYPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bt="([a-zA-Z]+)""#).unwrap());
static V_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<v>(.*?)</v>").unwrap());
static T_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<t[^>]*>(.*?)</t>").unwrap());
static SI_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<si>(.*?)</si>").unwrap());
static MERGE_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"ref="([A-Z]+)(\d+):([A-Z]+)(\d+)""#).unwrap());
static DIMENSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<dimension[^>]*/>").unwrap());

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn unescape(text: &str) -> String {
    // &amp; last, or "&amp;lt;" would decode twice into "<".
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The text a cell currently displays, resolving shared-string references.
fn cell_text(cell: &str, shared: &[String]) -> String {
    match ATTR_TYPE
        .captures(cell)
        .map(|c| c[1].to_string())
        .as_deref()
    {
        Some("s") => V_TAG
            .captures(cell)
            .and_then(|c| c[1].trim().parse::<usize>().ok())
            .and_then(|i| shared.get(i).cloned())
            .unwrap_or_default(),
        Some("inlineStr") => T_TAG
            .captures(cell)
            .map(|c| unescape(&c[1]))
            .unwrap_or_default(),
        Some("str") => V_TAG
            .captures(cell)
            .map(|c| unescape(&c[1]))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Rewrites one cell with `value`, keeping its `r` and - crucially - its `s`
/// style index, which is where the company's fonts and borders live.
fn write_cell(cell: &str, value: &Value) -> String {
    let reference = ATTR_REF
        .captures(cell)
        .map(|c| c[1].to_string())
        .unwrap_or_default();
    let style = ATTR_STYLE
        .captures(cell)
        .map(|c| format!(r#" s="{}""#, &c[1]))
        .unwrap_or_default();

    match value {
        // The digits come straight from the integer 分 - see `Value::Money`.
        // A numeric cell needs no `t` attribute; its absence IS the number
        // type, which is why this and Number share a shape.
        Value::Money(amount) => format!(
            r#"<c r="{reference}"{style}><v>{}</v></c>"#,
            amount.to_decimal_string()
        ),
        Value::Number(n) => format!(r#"<c r="{reference}"{style}><v>{n}</v></c>"#),
        Value::Text(text) if text.is_empty() => format!(r#"<c r="{reference}"{style}/>"#),
        Value::Text(text) => format!(
            // inlineStr rather than a shared string: cloning the detail row
            // means the same template cell needs a DIFFERENT value on every
            // copy, which one shared-string index cannot express.
            r#"<c r="{reference}"{style} t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#,
            escape(text)
        ),
    }
}

/// Substitutes bindings into every cell of one row.
///
/// A cell whose entire text is one placeholder takes that binding's type -
/// so `{{明细.合计}}` becomes a real number. A cell with a placeholder
/// embedded in a sentence ("申请人：{{申请人}}") stays text, because a number
/// inside a sentence is a sentence.
fn substitute_row(row: &str, shared: &[String], bindings: &HashMap<String, Value>) -> String {
    CELL.replace_all(row, |caps: &regex::Captures| {
        let cell = &caps[0];
        let text = cell_text(cell, shared);
        if !text.contains("{{") {
            return cell.to_string();
        }

        if let Some(value) = bindings.get(text.trim()) {
            return write_cell(cell, value);
        }

        let mut merged = text.clone();
        for (key, value) in bindings {
            if merged.contains(key.as_str()) {
                let rendered = match value {
                    Value::Text(t) => t.clone(),
                    Value::Number(n) => format!("{n}"),
                    Value::Money(amount) => amount.to_decimal_string(),
                };
                merged = merged.replace(key.as_str(), &rendered);
            }
        }
        write_cell(cell, &Value::Text(merged))
    })
    .into_owned()
}

/// Renumbers a row block to `target`, including every cell's `r` attribute.
fn renumber(row: &str, target: u32) -> String {
    // The <row> tag's own r="N" is purely numeric; cell refs are letters then
    // digits. Two patterns, so neither can match the other's attribute.
    let renumbered = ATTR_ROW_NUM.replace(row, format!(r#"r="{target}""#).as_str());
    ATTR_REF
        .replace_all(&renumbered, |caps: &regex::Captures| {
            let column: String = caps[1]
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            format!(r#"r="{column}{target}""#)
        })
        .into_owned()
}

fn row_number(row: &str) -> Option<u32> {
    ATTR_ROW_NUM.captures(row).and_then(|c| c[1].parse().ok())
}

/// Merged ranges below the detail row have to move with it, or the signature
/// block ends up merged across the wrong rows.
fn shift_merges(xml: &str, after: u32, by: i64) -> String {
    if by == 0 {
        return xml.to_string();
    }
    MERGE_REF
        .replace_all(xml, |caps: &regex::Captures| {
            let shift = |row: u32| -> u32 {
                if row > after {
                    (row as i64 + by).max(1) as u32
                } else {
                    row
                }
            };
            let top: u32 = caps[2].parse().unwrap_or(1);
            let bottom: u32 = caps[4].parse().unwrap_or(1);
            format!(
                r#"ref="{}{}:{}{}""#,
                &caps[1],
                shift(top),
                &caps[3],
                shift(bottom)
            )
        })
        .into_owned()
}

/// The result of filling a template, shown before the file is handed over.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FillReport {
    /// Placeholders the template contains, and which were filled.
    pub filled: Vec<String>,
    /// Placeholders the template does NOT contain. Not an error - a company
    /// form with no 可抵扣税额 column simply does not want one - but the user
    /// should see the list before mailing the sheet to finance.
    pub missing: Vec<String>,
    pub detail_rows: usize,
}

/// Fills `template_bytes` and writes the result to `out`.
pub fn fill(
    template_bytes: &[u8],
    invoices: &[Invoice],
    meta: &ReportMeta,
    out: &std::path::Path,
) -> Result<FillReport, String> {
    let summary = Summary::of(invoices);
    let scalars = scalar_bindings(&summary, meta);

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(template_bytes))
        .map_err(|_| "模板不是有效的 .xlsx 文件".to_string())?;

    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    let mut parts: Vec<(String, Vec<u8>)> = Vec::with_capacity(names.len());
    for name in &names {
        let mut buf = Vec::new();
        archive
            .by_name(name)
            .map_err(|e| format!("读取模板失败：{e}"))?
            .read_to_end(&mut buf)
            .map_err(|e| format!("读取模板失败：{e}"))?;
        parts.push((name.clone(), buf));
    }

    let shared = shared_strings(&parts);
    // The first worksheet. A mandated form is one sheet in practice, and
    // silently picking one of several would be worse than being predictable.
    let sheet_index = parts
        .iter()
        .position(|(name, _)| name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml"))
        .ok_or_else(|| "模板中找不到工作表".to_string())?;

    let sheet = String::from_utf8_lossy(&parts[sheet_index].1).into_owned();
    let (filled_sheet, report) = fill_sheet(&sheet, &shared, &scalars, invoices);
    parts[sheet_index].1 = filled_sheet.into_bytes();

    write_archive(&parts, out)?;
    Ok(report)
}

fn shared_strings(parts: &[(String, Vec<u8>)]) -> Vec<String> {
    let Some((_, bytes)) = parts
        .iter()
        .find(|(name, _)| name == "xl/sharedStrings.xml")
    else {
        return Vec::new();
    };
    let xml = String::from_utf8_lossy(bytes);
    // One <si> per string, but its text can be split across several <t> runs
    // when part of it was formatted differently - so they concatenate.
    SI_TAG
        .captures_iter(&xml)
        .map(|si| {
            T_TAG
                .captures_iter(&si[1])
                .map(|t| unescape(&t[1]))
                .collect::<String>()
        })
        .collect()
}

fn fill_sheet(
    sheet: &str,
    shared: &[String],
    scalars: &HashMap<String, Value>,
    invoices: &[Invoice],
) -> (String, FillReport) {
    let rows: Vec<&str> = ROW.find_iter(sheet).map(|m| m.as_str()).collect();

    let detail_index = rows.iter().position(|row| {
        CELL.find_iter(row)
            .any(|c| cell_text(c.as_str(), shared).contains("{{明细."))
    });

    // Which placeholders the template actually uses, computed before any
    // substitution replaces them.
    let all: Vec<String> = placeholders().iter().map(|(p, _)| p.to_string()).collect();
    let template_text: String = rows
        .iter()
        .flat_map(|row| CELL.find_iter(row))
        .map(|c| cell_text(c.as_str(), shared))
        .collect::<Vec<_>>()
        .join("\u{1}");
    let filled: Vec<String> = all
        .iter()
        .filter(|p| template_text.contains(p.as_str()))
        .cloned()
        .collect();
    let missing: Vec<String> = all
        .iter()
        .filter(|p| !template_text.contains(p.as_str()))
        .cloned()
        .collect();

    let mut out_rows: Vec<String> = Vec::new();
    let mut next_row: u32 = 1;
    let mut detail_rows = 0usize;
    let mut detail_template_row: u32 = 0;

    for (index, row) in rows.iter().enumerate() {
        if Some(index) == detail_index {
            detail_template_row = row_number(row).unwrap_or(next_row);
            // One clone per invoice, each carrying the template row's styling.
            for (position, invoice) in invoices.iter().enumerate() {
                let mut bindings = detail_bindings(invoice, position);
                bindings.extend(scalars.clone());
                let substituted = substitute_row(row, shared, &bindings);
                out_rows.push(renumber(&substituted, next_row));
                next_row += 1;
                detail_rows += 1;
            }
            // A report with no invoices DROPS the template row rather than
            // leaving `{{明细.日期}}` staring back from the finished sheet.
            continue;
        }

        let substituted = substitute_row(row, shared, scalars);
        out_rows.push(renumber(&substituted, next_row));
        next_row += 1;
    }

    let body = out_rows.join("");
    let mut result = match (ROW.find(sheet), ROW.find_iter(sheet).last()) {
        (Some(first), Some(last)) => {
            format!(
                "{}{}{}",
                &sheet[..first.start()],
                body,
                &sheet[last.end()..]
            )
        }
        // A template with no rows at all: nothing to substitute into.
        _ => sheet.to_string(),
    };

    if detail_index.is_some() {
        // Cloning N rows in place of 1 moves everything below down by N-1.
        result = shift_merges(&result, detail_template_row, invoices.len() as i64 - 1);
    }

    // `dimension` now describes the wrong range. Excel recomputes it on open
    // and a stale one confuses stricter readers, so it is dropped outright.
    let result = DIMENSION.replace(&result, "").into_owned();

    (
        result,
        FillReport {
            filled,
            missing,
            detail_rows,
        },
    )
}

fn write_archive(parts: &[(String, Vec<u8>)], out: &std::path::Path) -> Result<(), String> {
    let file = std::fs::File::create(out).map_err(|e| format!("无法写入文件：{e}"))?;
    let mut writer = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for (name, bytes) in parts {
        writer
            .start_file(name.as_str(), options)
            .map_err(|e| format!("无法写入文件：{e}"))?;
        writer
            .write_all(bytes)
            .map_err(|e| format!("无法写入文件：{e}"))?;
    }
    writer.finish().map_err(|e| format!("无法写入文件：{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Field, FieldSource, InvoiceKind};
    use rust_xlsxwriter::{Format, Workbook};

    fn invoice(seller: &str, category: &str, net: i64, tax: i64) -> Invoice {
        Invoice {
            kind: Some(InvoiceKind::DigitalGeneral),
            number: Field::new("24312000000012345678".into(), FieldSource::Xml),
            issued_on: Field::new("2024-03-01".into(), FieldSource::Xml),
            seller_name: Field::new(seller.into(), FieldSource::Xml),
            amount_excl_tax: Field::new(Money(net), FieldSource::Xml),
            tax: Field::new(Money(tax), FieldSource::Xml),
            total: Field::new(Money(net + tax), FieldSource::Xml),
            category: Some(category.into()),
            ..Default::default()
        }
    }

    /// A header with every scalar filled.
    ///
    /// The title matters to the tests that index rows: calamine reports a
    /// sheet's USED range, so an empty first row would silently shift every
    /// row index by one and make the assertions below test the wrong rows.
    pub(super) fn meta() -> ReportMeta {
        ReportMeta {
            title: "2024年3月报销单".into(),
            applicant: "沈".into(),
            department: "研发".into(),
            note: String::new(),
            date: "2024-04-01".into(),
        }
    }

    /// A stand-in for a company form: a title, a header row, one detail row
    /// of placeholders, and a total row below it. The bold format exists so a
    /// test can prove styling survives the round trip.
    pub(super) fn template() -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("模板.xlsx");
        let mut book = Workbook::new();
        let bold = Format::new().set_bold();
        let sheet = book.add_worksheet();
        sheet
            .write_string_with_format(0, 0, "{{标题}}", &bold)
            .unwrap();
        sheet.write_string(1, 0, "申请人：{{申请人}}").unwrap();
        sheet.write_string(2, 0, "序号").unwrap();
        sheet.write_string(2, 1, "销售方").unwrap();
        sheet.write_string(2, 2, "金额").unwrap();
        sheet
            .write_string_with_format(3, 0, "{{明细.序号}}", &bold)
            .unwrap();
        sheet.write_string(3, 1, "{{明细.销售方}}").unwrap();
        sheet.write_string(3, 2, "{{明细.合计}}").unwrap();
        sheet.write_string(4, 0, "合计").unwrap();
        sheet.write_string(4, 2, "{{合计}}").unwrap();
        sheet.write_string(5, 0, "大写：{{合计大写}}").unwrap();
        book.save(&path).unwrap();
        std::fs::read(&path).unwrap()
    }

    /// Reads a filled workbook back the way Excel would.
    fn read_back(path: &std::path::Path) -> Vec<Vec<String>> {
        use calamine::Reader;
        let mut book: calamine::Xlsx<_> =
            calamine::open_workbook(path).expect("生成的文件应能被读取");
        let name = book.sheet_names()[0].clone();
        let range = book.worksheet_range(&name).unwrap();
        range
            .rows()
            .map(|row| row.iter().map(|c| c.to_string()).collect())
            .collect()
    }

    /// Fills the stand-in company form, for tests in sibling modules.
    pub(super) fn fill_for_test(invoices: &[Invoice], out: &std::path::Path) {
        fill(&template(), invoices, &meta(), out).expect("模板填充应成功");
    }

    #[test]
    fn scalar_placeholders_are_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.xlsx");
        fill(
            &template(),
            &[invoice("某某酒店", "住宿", 100_000, 6_000)],
            &ReportMeta {
                title: "2024年3月报销单".into(),
                applicant: "沈".into(),
                ..Default::default()
            },
            &out,
        )
        .unwrap();

        let rows = read_back(&out);
        assert_eq!(rows[0][0], "2024年3月报销单");
        // A placeholder embedded in a sentence.
        assert_eq!(rows[1][0], "申请人：沈");
    }

    /// The core of the feature: one template row becomes N rows.
    #[test]
    fn the_detail_row_is_cloned_once_per_invoice() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.xlsx");
        let invoices = vec![
            invoice("某某酒店", "住宿", 100_000, 6_000),
            invoice("某某餐厅", "餐饮", 50_000, 3_000),
            invoice("某某科技", "软件与服务", 20_000, 1_200),
        ];
        let report = fill(&template(), &invoices, &meta(), &out).unwrap();
        assert_eq!(report.detail_rows, 3);

        let rows = read_back(&out);
        assert_eq!(rows[3][1], "某某酒店");
        assert_eq!(rows[4][1], "某某餐厅");
        assert_eq!(rows[5][1], "某某科技");
        // The rows below moved down to make room rather than being overwritten.
        assert_eq!(rows[6][0], "合计");
    }

    /// Money must arrive as a NUMBER, or `SUM` over the column returns zero
    /// and the sheet is quietly useless.
    #[test]
    fn amounts_are_written_as_numbers_not_text() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.xlsx");
        fill(
            &template(),
            &[invoice("某某酒店", "住宿", 100_000, 6_000)],
            &meta(),
            &out,
        )
        .unwrap();

        use calamine::{Data, Reader};
        let mut book: calamine::Xlsx<_> = calamine::open_workbook(&out).unwrap();
        let name = book.sheet_names()[0].clone();
        let range = book.worksheet_range(&name).unwrap();
        let cell = range.get((3, 2)).expect("明细金额单元格");
        assert!(
            matches!(cell, Data::Float(_) | Data::Int(_)),
            "金额应为数字，实际是 {cell:?}"
        );
    }

    #[test]
    fn the_capital_amount_is_rendered() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.xlsx");
        fill(
            &template(),
            &[invoice("某某酒店", "住宿", 100_000, 6_000)],
            &meta(),
            &out,
        )
        .unwrap();
        let rows = read_back(&out);
        assert_eq!(rows.last().unwrap()[0], "大写：壹仟零陆拾圆整");
    }

    /// A form with no invoices must not ship `{{明细.序号}}` to finance.
    #[test]
    fn an_empty_report_drops_the_template_row() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.xlsx");
        let report = fill(&template(), &[], &ReportMeta::default(), &out).unwrap();
        assert_eq!(report.detail_rows, 0);

        let flat = read_back(&out).concat().join("|");
        assert!(!flat.contains("{{"), "残留占位符: {flat}");
    }

    /// Silence about an unfilled placeholder is how a sheet reaches finance
    /// missing a column nobody noticed.
    #[test]
    fn unused_placeholders_are_reported_not_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.xlsx");
        let report = fill(
            &template(),
            &[invoice("某某酒店", "住宿", 100_000, 6_000)],
            &meta(),
            &out,
        )
        .unwrap();

        assert!(report.filled.contains(&"{{标题}}".to_string()));
        assert!(report.filled.contains(&"{{明细.销售方}}".to_string()));
        assert!(report.missing.contains(&"{{可抵扣税额}}".to_string()));
    }

    #[test]
    fn a_file_that_is_not_an_xlsx_is_rejected_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.xlsx");
        let error = fill(b"not a workbook", &[], &ReportMeta::default(), &out).unwrap_err();
        assert!(error.contains("xlsx"), "{error}");
    }

    #[test]
    fn special_characters_in_a_seller_name_survive_escaping() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.xlsx");
        fill(
            &template(),
            &[invoice("A&B <科技> 有限公司", "软件与服务", 10_000, 600)],
            &meta(),
            &out,
        )
        .unwrap();
        assert_eq!(read_back(&out)[3][1], "A&B <科技> 有限公司");
    }
}

#[cfg(test)]
mod money_path_tests {
    use super::*;
    use crate::model::{Field, FieldSource, InvoiceKind};

    /// The invariant behind `Value::Money`: the digits written into the sheet
    /// XML are produced from the integer 分 by integer division, so they are
    /// exactly what the ledger holds - no float, no rounding, no drift.
    ///
    /// The amounts chosen are the ones a naive `cents as f64 / 100.0` plus
    /// float formatting is most likely to render as `0.07000000000000001` or
    /// to drop a trailing zero from.
    #[test]
    fn amounts_reach_the_cell_xml_as_exact_decimals() {
        for (cents, expected) in [
            (7i64, "0.07"),
            (10, "0.10"),
            (105, "1.05"),
            (4_350, "43.50"),
            (106_000, "1060.00"),
            (1_234_567, "12345.67"),
            (9_999_999_999, "99999999.99"),
        ] {
            let cell = write_cell(r#"<c r="C4" s="3"/>"#, &Value::Money(Money(cents)));
            assert!(
                cell.contains(&format!("<v>{expected}</v>")),
                "{cents} 分应写成 {expected}，实际是 {cell}"
            );
            // No `t` attribute means "number" - a `t="inlineStr"` here would
            // make SUM skip the cell.
            assert!(!cell.contains("t="), "金额单元格不该有类型属性: {cell}");
            // The company's own styling must survive.
            assert!(cell.contains(r#"s="3""#), "样式索引丢失: {cell}");
        }
    }

    /// A money placeholder inside a sentence still renders exactly, even
    /// though it becomes text there.
    #[test]
    fn money_embedded_in_text_keeps_its_two_decimals() {
        let mut bindings = HashMap::new();
        bindings.insert("{{合计}}".to_string(), Value::Money(Money(4_350)));
        let row =
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>共计 {{合计}} 元</t></is></c></row>"#;
        let filled = substitute_row(row, &[], &bindings);
        assert!(filled.contains("共计 43.50 元"), "{filled}");
    }

    /// End to end: the number that lands in the file is the number in the
    /// ledger, read back the way Excel would read it.
    #[test]
    fn the_exported_amount_equals_the_stored_cents() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.xlsx");
        let invoice = Invoice {
            kind: Some(InvoiceKind::DigitalGeneral),
            number: Field::new("24312000000012345678".into(), FieldSource::Xml),
            issued_on: Field::new("2024-03-01".into(), FieldSource::Xml),
            seller_name: Field::new("某某公司".into(), FieldSource::Xml),
            total: Field::new(Money(4_350), FieldSource::Xml),
            ..Default::default()
        };

        super::tests::fill_for_test(&[invoice], &out);

        use calamine::{Data, Reader};
        let mut book: calamine::Xlsx<_> = calamine::open_workbook(&out).unwrap();
        let name = book.sheet_names()[0].clone();
        let range = book.worksheet_range(&name).unwrap();
        match range.get((3, 2)).expect("明细金额单元格") {
            Data::Float(value) => {
                assert_eq!((value * 100.0).round() as i64, 4_350, "读回来的金额对不上");
            }
            Data::Int(value) => assert_eq!(*value * 100, 4_350),
            other => panic!("金额应为数字，实际是 {other:?}"),
        }
    }
}
