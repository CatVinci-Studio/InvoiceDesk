//! Producing the thing the user actually hands to finance: a 报销单.
//!
//! Two ways out, because Chinese companies split cleanly into two camps:
//!
//! - [`workbook`] writes a clean, self-contained .xlsx - 明细表 plus 分类
//!   汇总表 - for anyone who does not have a mandated form.
//! - [`template`] fills in the company's OWN .xlsx, preserving its
//!   formatting, for everyone who does.
//!
//! Both share [`Summary`], so the totals on a company form and the totals on
//! the generic sheet are computed by the same code and cannot disagree.

pub mod template;

use crate::classify::UNCATEGORISED;
use crate::model::{Invoice, Money};
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, Workbook};
use serde::{Deserialize, Serialize};

/// The header block a reimbursement form carries above the table.
///
/// `default` for the same reason [`crate::db::Filter`] has it: an export with
/// no 部门 filled in is an ordinary export, not a malformed request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReportMeta {
    pub title: String,
    pub applicant: String,
    pub department: String,
    pub note: String,
    /// `YYYY-MM-DD`; the date the sheet was produced.
    pub date: String,
}

/// One row of 分类汇总.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryLine {
    pub category: String,
    pub count: usize,
    pub amount_excl_tax: Money,
    pub tax: Money,
    pub total: Money,
}

/// Everything derived from a set of invoices, computed once.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub count: usize,
    pub amount_excl_tax: Money,
    pub tax: Money,
    pub total: Money,
    /// 可抵扣进项税额 - only 专用发票 contribute. Finance asks for this
    /// separately from the total tax, because it is the part the company can
    /// actually reclaim.
    pub deductible_tax: Money,
    pub by_category: Vec<CategoryLine>,
}

impl Summary {
    pub fn of(invoices: &[Invoice]) -> Summary {
        let mut summary = Summary {
            count: invoices.len(),
            ..Default::default()
        };

        for invoice in invoices {
            let total = invoice.total.value.unwrap_or(Money::ZERO);
            let tax = invoice.tax.value.unwrap_or(Money::ZERO);
            let net = invoice
                .amount_excl_tax
                .value
                // Not every 票种 prints a tax breakdown (train tickets,
                // fixed-amount receipts). Treating the whole amount as
                // tax-exclusive there keeps 明细 columns summing to 合计.
                .unwrap_or(total - tax);

            summary.total = summary.total + total;
            summary.tax = summary.tax + tax;
            summary.amount_excl_tax = summary.amount_excl_tax + net;
            if invoice.kind.is_some_and(|k| k.is_deductible()) {
                summary.deductible_tax = summary.deductible_tax + tax;
            }

            let category = invoice
                .category
                .clone()
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| UNCATEGORISED.to_string());
            match summary
                .by_category
                .iter_mut()
                .find(|l| l.category == category)
            {
                Some(line) => {
                    line.count += 1;
                    line.amount_excl_tax = line.amount_excl_tax + net;
                    line.tax = line.tax + tax;
                    line.total = line.total + total;
                }
                None => summary.by_category.push(CategoryLine {
                    category,
                    count: 1,
                    amount_excl_tax: net,
                    tax,
                    total,
                }),
            }
        }

        // Largest first: that is the order the person approving this reads in.
        summary.by_category.sort_by(|a, b| {
            b.total
                .cmp(&a.total)
                .then_with(|| a.category.cmp(&b.category))
        });
        summary
    }
}

// ---------------------------------------------------------------------------
// 人民币大写
// ---------------------------------------------------------------------------

/// The amount in 中文大写, e.g. `壹仟零陆拾圆整`.
///
/// Every Chinese reimbursement form has this field, and every implementation
/// of it gets the zeros wrong somewhere, so the rules are spelled out:
///
/// - consecutive zeros collapse to a single 零
/// - a zero at the end of the 圆 part is dropped (壹仟零陆拾, not 壹仟零陆拾零)
/// - 整 is appended when there are no 分 (and no 角)
/// - when 角 is zero but 分 is not, a 零 stands in for it (壹圆零伍分)
pub fn chinese_capital(amount: Money) -> String {
    const DIGITS: [&str; 10] = ["零", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖"];
    const PLACES: [&str; 4] = ["", "拾", "佰", "仟"];
    const GROUPS: [&str; 5] = ["", "万", "亿", "万亿", "亿亿"];

    let cents = amount.cents();
    let sign = if cents < 0 { "负" } else { "" };
    let cents = cents.abs();

    let (yuan, jiao, fen) = (cents / 100, (cents / 10) % 10, cents % 10);

    let mut integer = String::new();
    if yuan > 0 {
        // Split into 4-digit groups, least significant first.
        let mut groups = Vec::new();
        let mut rest = yuan;
        while rest > 0 {
            groups.push(rest % 10_000);
            rest /= 10_000;
        }

        for (index, group) in groups.iter().enumerate().rev() {
            if *group == 0 {
                // An all-zero group still needs to be audible: 壹亿零壹万.
                if !integer.is_empty() && !integer.ends_with('零') {
                    integer.push('零');
                }
                continue;
            }

            // A group under 1000 has a leading zero within the number
            // (壹万零壹佰), which the group's own digits cannot express.
            if *group < 1000 && !integer.is_empty() && !integer.ends_with('零') {
                integer.push('零');
            }

            let mut pending_zero = false;
            let mut started = false;
            for place in (0..4).rev() {
                let digit = (group / 10_i64.pow(place)) % 10;
                if digit == 0 {
                    pending_zero = started;
                } else {
                    if pending_zero {
                        integer.push('零');
                        pending_zero = false;
                    }
                    integer.push_str(DIGITS[digit as usize]);
                    integer.push_str(PLACES[place as usize]);
                    started = true;
                }
            }
            integer.push_str(GROUPS[index]);
        }

        // 壹亿零 → 壹亿: a trailing 零 belongs to nothing.
        while integer.ends_with('零') {
            integer.pop();
        }
        integer.push('圆');
    }

    let fraction = match (jiao, fen) {
        (0, 0) => "整".to_string(),
        (j, 0) => format!("{}角整", DIGITS[j as usize]),
        // 零 stands in for the missing 角 only when there is a 圆 part in
        // front of it - "零伍分" on its own would read as nonsense.
        (0, f) if yuan > 0 => format!("零{}分", DIGITS[f as usize]),
        (0, f) => format!("{}分", DIGITS[f as usize]),
        (j, f) => format!("{}角{}分", DIGITS[j as usize], DIGITS[f as usize]),
    };

    if integer.is_empty() && cents == 0 {
        return "零圆整".to_string();
    }
    format!("{sign}{integer}{fraction}")
}

/// Money as a spreadsheet number, so the cell can be summed and formatted by
/// Excel rather than being text that merely looks like a number.
///
/// The float conversion lives on [`Money::to_yuan_f64`], which documents why
/// it is safe and why this is the only place it is allowed to happen. A
/// missing amount becomes 0 rather than a blank cell: a gap in a summed
/// column reads as "not applicable" to some spreadsheet readers and breaks
/// the total in others.
fn yuan(amount: Option<Money>) -> f64 {
    amount.unwrap_or(Money::ZERO).to_yuan_f64()
}

// ---------------------------------------------------------------------------
// The generic workbook
// ---------------------------------------------------------------------------

const HEADERS: [(&str, f64); 11] = [
    ("序号", 6.0),
    ("开票日期", 12.0),
    ("发票号码", 24.0),
    ("票种", 20.0),
    ("销售方名称", 30.0),
    ("费用类别", 12.0),
    ("金额(不含税)", 14.0),
    ("税额", 12.0),
    ("价税合计", 14.0),
    ("可抵扣", 8.0),
    ("备注/问题", 30.0),
];

/// Writes 明细表 + 汇总表 to `path`.
pub fn workbook(
    invoices: &[Invoice],
    meta: &ReportMeta,
    path: &std::path::Path,
) -> Result<(), String> {
    let summary = Summary::of(invoices);
    let mut book = Workbook::new();

    let title = Format::new()
        .set_bold()
        .set_font_size(15.0)
        .set_align(FormatAlign::Center);
    let label = Format::new().set_bold();
    let header = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xF4F2EE))
        .set_border(FormatBorder::Thin)
        .set_align(FormatAlign::Center);
    let cell = Format::new().set_border(FormatBorder::Thin);
    let money = Format::new()
        .set_border(FormatBorder::Thin)
        .set_num_format("#,##0.00");
    let money_bold = Format::new()
        .set_border(FormatBorder::Thin)
        .set_num_format("#,##0.00")
        .set_bold();
    // Counts share the bold total-row look but keep the general format, so a
    // 张数 of 128 does not come out as "128.00".
    let count_bold = Format::new().set_border(FormatBorder::Thin).set_bold();
    let warn = Format::new()
        .set_border(FormatBorder::Thin)
        .set_font_color(Color::RGB(0xC1495A));

    // --- 明细表 ---
    let sheet = book.add_worksheet();
    sheet.set_name("报销明细").map_err(err)?;

    let last_col = (HEADERS.len() - 1) as u16;
    sheet
        .merge_range(0, 0, 0, last_col, &meta.title, &title)
        .map_err(err)?;
    sheet
        .write_string_with_format(1, 0, "申请人", &label)
        .map_err(err)?;
    sheet.write_string(1, 1, &meta.applicant).map_err(err)?;
    sheet
        .write_string_with_format(1, 3, "部门", &label)
        .map_err(err)?;
    sheet.write_string(1, 4, &meta.department).map_err(err)?;
    sheet
        .write_string_with_format(1, 6, "制表日期", &label)
        .map_err(err)?;
    sheet.write_string(1, 7, &meta.date).map_err(err)?;

    const TABLE_TOP: u32 = 3;
    for (col, (name, width)) in HEADERS.iter().enumerate() {
        sheet
            .write_string_with_format(TABLE_TOP, col as u16, *name, &header)
            .map_err(err)?;
        sheet.set_column_width(col as u16, *width).map_err(err)?;
    }

    for (index, invoice) in invoices.iter().enumerate() {
        let row = TABLE_TOP + 1 + index as u32;
        let issues = invoice
            .issues
            .iter()
            .map(|i| i.message())
            .collect::<Vec<_>>()
            .join("；");

        sheet
            .write_number_with_format(row, 0, (index + 1) as f64, &cell)
            .map_err(err)?;
        sheet
            .write_string_with_format(
                row,
                1,
                invoice.issued_on.as_ref().map(String::as_str).unwrap_or(""),
                &cell,
            )
            .map_err(err)?;
        sheet
            .write_string_with_format(
                row,
                2,
                invoice.number.as_ref().map(String::as_str).unwrap_or(""),
                &cell,
            )
            .map_err(err)?;
        sheet
            .write_string_with_format(row, 3, invoice.kind.map(|k| k.label()).unwrap_or(""), &cell)
            .map_err(err)?;
        sheet
            .write_string_with_format(
                row,
                4,
                invoice
                    .seller_name
                    .as_ref()
                    .map(String::as_str)
                    .unwrap_or(""),
                &cell,
            )
            .map_err(err)?;
        sheet
            .write_string_with_format(
                row,
                5,
                invoice.category.as_deref().unwrap_or(UNCATEGORISED),
                &cell,
            )
            .map_err(err)?;
        sheet
            .write_number_with_format(row, 6, yuan(invoice.amount_excl_tax.value), &money)
            .map_err(err)?;
        sheet
            .write_number_with_format(row, 7, yuan(invoice.tax.value), &money)
            .map_err(err)?;
        sheet
            .write_number_with_format(row, 8, yuan(invoice.total.value), &money)
            .map_err(err)?;
        sheet
            .write_string_with_format(
                row,
                9,
                if invoice.kind.is_some_and(|k| k.is_deductible()) {
                    "是"
                } else {
                    ""
                },
                &cell,
            )
            .map_err(err)?;
        // Problems ride into the export in red rather than being dropped:
        // whoever approves this needs to see that a total did not add up.
        sheet
            .write_string_with_format(
                row,
                10,
                &issues,
                if issues.is_empty() { &cell } else { &warn },
            )
            .map_err(err)?;
    }

    let total_row = TABLE_TOP + 1 + invoices.len() as u32;
    sheet
        .write_string_with_format(total_row, 0, "合计", &header)
        .map_err(err)?;
    for col in 1..=5 {
        sheet
            .write_string_with_format(total_row, col, "", &header)
            .map_err(err)?;
    }
    sheet
        .write_number_with_format(
            total_row,
            6,
            yuan(Some(summary.amount_excl_tax)),
            &money_bold,
        )
        .map_err(err)?;
    sheet
        .write_number_with_format(total_row, 7, yuan(Some(summary.tax)), &money_bold)
        .map_err(err)?;
    sheet
        .write_number_with_format(total_row, 8, yuan(Some(summary.total)), &money_bold)
        .map_err(err)?;
    sheet
        .write_string_with_format(total_row, 9, "", &header)
        .map_err(err)?;
    sheet
        .write_string_with_format(total_row, 10, "", &header)
        .map_err(err)?;

    let capital_row = total_row + 2;
    sheet
        .write_string_with_format(capital_row, 0, "合计金额（大写）", &label)
        .map_err(err)?;
    sheet
        .merge_range(
            capital_row,
            1,
            capital_row,
            4,
            &chinese_capital(summary.total),
            &label,
        )
        .map_err(err)?;
    if !meta.note.is_empty() {
        sheet
            .write_string_with_format(capital_row + 1, 0, "说明", &label)
            .map_err(err)?;
        sheet
            .write_string(capital_row + 1, 1, &meta.note)
            .map_err(err)?;
    }

    // --- 汇总表 ---
    let sheet = book.add_worksheet();
    sheet.set_name("分类汇总").map_err(err)?;
    for (col, (name, width)) in [
        ("费用类别", 16.0),
        ("张数", 8.0),
        ("金额(不含税)", 14.0),
        ("税额", 12.0),
        ("价税合计", 14.0),
        ("占比", 10.0),
    ]
    .iter()
    .enumerate()
    {
        sheet
            .write_string_with_format(0, col as u16, *name, &header)
            .map_err(err)?;
        sheet.set_column_width(col as u16, *width).map_err(err)?;
    }

    let percent = Format::new()
        .set_border(FormatBorder::Thin)
        .set_num_format("0.0%");
    for (index, line) in summary.by_category.iter().enumerate() {
        let row = 1 + index as u32;
        let share = if summary.total.cents() == 0 {
            0.0
        } else {
            line.total.cents() as f64 / summary.total.cents() as f64
        };
        sheet
            .write_string_with_format(row, 0, &line.category, &cell)
            .map_err(err)?;
        sheet
            .write_number_with_format(row, 1, line.count as f64, &cell)
            .map_err(err)?;
        sheet
            .write_number_with_format(row, 2, yuan(Some(line.amount_excl_tax)), &money)
            .map_err(err)?;
        sheet
            .write_number_with_format(row, 3, yuan(Some(line.tax)), &money)
            .map_err(err)?;
        sheet
            .write_number_with_format(row, 4, yuan(Some(line.total)), &money)
            .map_err(err)?;
        sheet
            .write_number_with_format(row, 5, share, &percent)
            .map_err(err)?;
    }

    let row = 1 + summary.by_category.len() as u32;
    sheet
        .write_string_with_format(row, 0, "合计", &header)
        .map_err(err)?;
    // A COUNT, not an amount - `money_bold` would render 128 张 as "128.00".
    // The number formats are the one place the two kinds of figure have to be
    // told apart in this sheet, since both are written as numbers.
    sheet
        .write_number_with_format(row, 1, summary.count as f64, &count_bold)
        .map_err(err)?;
    sheet
        .write_number_with_format(row, 2, yuan(Some(summary.amount_excl_tax)), &money_bold)
        .map_err(err)?;
    sheet
        .write_number_with_format(row, 3, yuan(Some(summary.tax)), &money_bold)
        .map_err(err)?;
    sheet
        .write_number_with_format(row, 4, yuan(Some(summary.total)), &money_bold)
        .map_err(err)?;
    sheet
        .write_string_with_format(row, 5, "", &header)
        .map_err(err)?;

    sheet
        .write_string_with_format(row + 2, 0, "其中可抵扣进项税额", &label)
        .map_err(err)?;
    sheet
        .write_number_with_format(row + 2, 2, yuan(Some(summary.deductible_tax)), &money_bold)
        .map_err(err)?;

    book.save(path).map_err(err)?;
    Ok(())
}

fn err(e: rust_xlsxwriter::XlsxError) -> String {
    format!("生成 Excel 失败：{e}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Field, FieldSource, InvoiceKind};

    fn invoice(category: &str, kind: InvoiceKind, net: i64, tax: i64) -> Invoice {
        Invoice {
            kind: Some(kind),
            number: Field::new("24312000000012345678".into(), FieldSource::Xml),
            issued_on: Field::new("2024-03-01".into(), FieldSource::Xml),
            seller_name: Field::new("某某公司".into(), FieldSource::Xml),
            amount_excl_tax: Field::new(Money(net), FieldSource::Xml),
            tax: Field::new(Money(tax), FieldSource::Xml),
            total: Field::new(Money(net + tax), FieldSource::Xml),
            category: Some(category.into()),
            ..Default::default()
        }
    }

    #[test]
    fn capital_amounts_follow_the_zero_rules() {
        let cases = [
            (0, "零圆整"),
            (100, "壹圆整"),
            (106_000, "壹仟零陆拾圆整"),
            (1_000, "壹拾圆整"),
            (50, "伍角整"),
            (5, "伍分"),
            (105, "壹圆零伍分"),
            (156, "壹圆伍角陆分"),
            (150, "壹圆伍角整"),
            (1_000_100, "壹万零壹圆整"),
            (11_000_000, "壹拾壹万圆整"),
            (10_000_000_000, "壹亿圆整"),
            (-8_800, "负捌拾捌圆整"),
        ];
        for (cents, expected) in cases {
            assert_eq!(chinese_capital(Money(cents)), expected, "{cents} 分");
        }
    }

    /// The invariant finance checks first: the 明细 columns have to add up to
    /// the 合计 row.
    #[test]
    fn the_columns_sum_to_the_total() {
        let invoices = vec![
            invoice("住宿", InvoiceKind::DigitalGeneral, 100_000, 6_000),
            invoice("餐饮", InvoiceKind::VatGeneral, 50_000, 3_000),
        ];
        let summary = Summary::of(&invoices);
        assert_eq!(
            summary.amount_excl_tax + summary.tax,
            summary.total,
            "不含税 + 税额 应等于价税合计"
        );
        assert_eq!(summary.total, Money(159_000));
    }

    /// Train tickets print no tax breakdown; the derived net must still make
    /// the columns balance rather than leaving a hole.
    #[test]
    fn receipts_without_a_tax_breakdown_still_balance() {
        let mut ticket = invoice("长途交通", InvoiceKind::Train, 0, 0);
        ticket.amount_excl_tax = Field::default();
        ticket.tax = Field::default();
        ticket.total = Field::new(Money(55_350), FieldSource::QrCode);

        let summary = Summary::of(&[ticket]);
        assert_eq!(summary.amount_excl_tax, Money(55_350));
        assert_eq!(summary.amount_excl_tax + summary.tax, summary.total);
    }

    /// Only 专用发票 tax is reclaimable; counting 普通发票 tax would overstate
    /// what the company can deduct.
    #[test]
    fn only_special_invoices_count_as_deductible() {
        let invoices = vec![
            invoice("住宿", InvoiceKind::DigitalSpecial, 100_000, 6_000),
            invoice("餐饮", InvoiceKind::VatGeneral, 50_000, 3_000),
        ];
        let summary = Summary::of(&invoices);
        assert_eq!(summary.tax, Money(9_000));
        assert_eq!(summary.deductible_tax, Money(6_000));
    }

    #[test]
    fn categories_are_summarised_largest_first() {
        let invoices = vec![
            invoice("餐饮", InvoiceKind::VatGeneral, 10_000, 0),
            invoice("住宿", InvoiceKind::VatGeneral, 90_000, 0),
            invoice("餐饮", InvoiceKind::VatGeneral, 20_000, 0),
        ];
        let summary = Summary::of(&invoices);
        assert_eq!(summary.by_category[0].category, "住宿");
        assert_eq!(summary.by_category[1].category, "餐饮");
        assert_eq!(summary.by_category[1].count, 2);
        assert_eq!(summary.by_category[1].total, Money(30_000));
    }

    #[test]
    fn invoices_with_no_category_are_grouped_not_dropped() {
        let mut orphan = invoice("", InvoiceKind::VatGeneral, 10_000, 0);
        orphan.category = None;
        let summary = Summary::of(&[orphan]);
        assert_eq!(summary.by_category[0].category, UNCATEGORISED);
        assert_eq!(summary.count, 1);
    }

    #[test]
    fn writes_a_readable_workbook() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("报销单.xlsx");
        let invoices = vec![
            invoice("住宿", InvoiceKind::DigitalSpecial, 100_000, 6_000),
            invoice("餐饮", InvoiceKind::VatGeneral, 50_000, 3_000),
        ];
        workbook(
            &invoices,
            &ReportMeta {
                title: "2024年3月差旅报销单".into(),
                applicant: "沈".into(),
                department: "研发".into(),
                note: String::new(),
                date: "2024-04-01".into(),
            },
            &path,
        )
        .unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"PK"), "xlsx 应是 zip");
        assert!(bytes.len() > 2_000);
    }

    #[test]
    fn an_empty_report_still_produces_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("空.xlsx");
        workbook(&[], &ReportMeta::default(), &path).unwrap();
        assert!(path.exists());
    }
}
