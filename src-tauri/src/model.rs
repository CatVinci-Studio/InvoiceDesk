//! The invoice domain model.
//!
//! Two decisions here shape everything downstream:
//!
//! 1. **Money is integer 分 (cents), never a float.** A reimbursement sheet
//!    that is off by a hundredth of a yuan is a defect, and binary floats
//!    cannot represent 0.1 exactly. [`Money`] is an `i64` count of cents,
//!    which covers ±92 quadrillion yuan - comfortably more than any invoice.
//!
//! 2. **Every field carries where it came from and how much to trust it.**
//!    The extraction pipeline has layers of wildly different reliability: an
//!    attached invoice XML is authoritative, a QR code is exact-or-absent, a
//!    PDF text layer is usually right, and a vision model is a good guess.
//!    Collapsing all four into a bare `String` would throw away the only
//!    thing that lets the UI say "check this number yourself" - so each
//!    value is a [`Field`], and the UI flags anything below
//!    [`REVIEW_THRESHOLD`].

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Money
// ---------------------------------------------------------------------------

/// An amount in 分 (hundredths of a yuan).
///
/// Serialises as a JSON number of cents, not yuan - the frontend formats it
/// for display but never does arithmetic on a fractional value either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Money(pub i64);

impl Money {
    pub const ZERO: Money = Money(0);

    pub fn from_cents(cents: i64) -> Self {
        Money(cents)
    }

    pub fn cents(self) -> i64 {
        self.0
    }

    /// The amount as a plain decimal string: `106000` → `"1060.00"`.
    ///
    /// Built by integer division, never by formatting a float. This is what
    /// goes into a spreadsheet cell's XML, so it is the exact text a user
    /// sees and a formula sums - there is no rounding step between the
    /// integer 分 stored in the ledger and the characters written to disk.
    ///
    /// Always two decimals, including for whole yuan, because a column of
    /// `1060` next to `43.50` reads as a different kind of number.
    pub fn to_decimal_string(self) -> String {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        format!("{sign}{}.{:02}", abs / 100, abs % 100)
    }

    /// The amount in yuan as an `f64`.
    ///
    /// **This is the only place money is allowed to become a float, and it
    /// exists because of a constraint in the file format rather than a
    /// choice.** A numeric cell in .xlsx is an IEEE double by specification -
    /// `rust_xlsxwriter::write_number` takes an `f64` and there is no
    /// decimal-typed cell to write instead. Writing the amount as *text*
    /// would dodge the float and break `SUM`, which is worse.
    ///
    /// It is safe here, and the reason is worth being precise about rather
    /// than hand-waving: `n / 100.0` for any `n` up to 2^53 is a single
    /// correctly-rounded division, so the result is the double nearest the
    /// true value, and every such double converts back to exactly `n` (see
    /// `every_cent_survives_the_xlsx_round_trip`). 2^53 分 is ninety
    /// trillion yuan. What must never happen is money being *accumulated* in
    /// f64 - that is where the errors compound - which is why `Summary` adds
    /// in `Money` and calls this once, at the very last step.
    ///
    /// The template exporter does not use this at all: it writes
    /// [`Self::to_decimal_string`] straight into the cell XML, so that path
    /// has no float in it whatsoever.
    pub fn to_yuan_f64(self) -> f64 {
        self.0 as f64 / 100.0
    }

    /// Parses the amount forms that actually appear on Chinese invoices:
    /// `1234.56`, `¥1234.56`, `￥1,234.56`, `1,234.56`, `(小写)1234.56`.
    ///
    /// Returns None rather than 0 on anything unrecognised - a silent zero
    /// would quietly shrink a reimbursement total, which is exactly the
    /// class of error this whole module exists to prevent.
    pub fn parse(raw: &str) -> Option<Money> {
        let cleaned: String = raw
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
            .collect();
        if cleaned.is_empty() {
            return None;
        }

        let negative = cleaned.starts_with('-');
        let digits = cleaned.trim_start_matches('-');
        // A second '.' means we picked up two numbers stuck together, not one
        // amount - refuse instead of parsing a prefix and being confidently
        // wrong about the value.
        if digits.matches('.').count() > 1 || digits.is_empty() {
            return None;
        }

        let (yuan_part, frac_part) = match digits.split_once('.') {
            Some((y, f)) => (y, f),
            None => (digits, ""),
        };
        if yuan_part.is_empty() && frac_part.is_empty() {
            return None;
        }

        let yuan: i64 = if yuan_part.is_empty() {
            0
        } else {
            yuan_part.parse().ok()?
        };
        // Round to 分 rather than truncating: invoice line items occasionally
        // carry more precision than the total does.
        let cents: i64 = match frac_part.len() {
            0 => 0,
            1 => frac_part.parse::<i64>().ok()? * 10,
            2 => frac_part.parse().ok()?,
            _ => {
                let hundredths: i64 = frac_part[..2].parse().ok()?;
                let next = frac_part.as_bytes()[2] - b'0';
                hundredths + if next >= 5 { 1 } else { 0 }
            }
        };

        let total = yuan.checked_mul(100)?.checked_add(cents)?;
        Some(Money(if negative { -total } else { total }))
    }
}

impl fmt::Display for Money {
    /// `¥1234.56` - no thousands separators, because this string also lands
    /// in spreadsheet cells where a comma would fight the cell format.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.abs();
        write!(f, "{sign}¥{}.{:02}", abs / 100, abs % 100)
    }
}

impl std::ops::Add for Money {
    type Output = Money;
    fn add(self, rhs: Money) -> Money {
        Money(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Money {
    type Output = Money;
    fn sub(self, rhs: Money) -> Money {
        Money(self.0 - rhs.0)
    }
}

impl std::iter::Sum for Money {
    fn sum<I: Iterator<Item = Money>>(iter: I) -> Money {
        Money(iter.map(|m| m.0).sum())
    }
}

// ---------------------------------------------------------------------------
// Field provenance
// ---------------------------------------------------------------------------

/// Where a field's value came from, ordered worst-to-best so `>` means
/// "more trustworthy" and a merge can keep the better of two readings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldSource {
    /// Nothing found this field.
    Missing,
    /// A vision model read it off a photo. A good guess, never a fact.
    Vision,
    /// Regex over a PDF's text layer. Right when the layout is standard.
    PdfText,
    /// Decoded from the invoice's QR code. Exact or absent, never wrong-ish.
    QrCode,
    /// Parsed from structured invoice XML (a PDF attachment, or an OFD part).
    /// This IS the invoice as the tax system issued it.
    Xml,
    /// The user typed it during review. Beats every machine reading, and is
    /// never overwritten by a re-scan.
    Manual,
}

impl FieldSource {
    /// The confidence a fresh reading from this source starts at.
    pub fn base_confidence(self) -> f32 {
        match self {
            FieldSource::Missing => 0.0,
            FieldSource::Vision => 0.75,
            FieldSource::PdfText => 0.90,
            FieldSource::QrCode => 0.95,
            FieldSource::Xml => 1.0,
            FieldSource::Manual => 1.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FieldSource::Missing => "未识别",
            FieldSource::Vision => "AI 识别",
            FieldSource::PdfText => "PDF 文本层",
            FieldSource::QrCode => "二维码",
            FieldSource::Xml => "发票 XML",
            FieldSource::Manual => "人工录入",
        }
    }
}

/// Below this, the UI marks the field for human review before the invoice can
/// go into a reimbursement sheet.
pub const REVIEW_THRESHOLD: f32 = 0.90;

/// One extracted value plus its provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field<T> {
    pub value: Option<T>,
    pub confidence: f32,
    pub source: FieldSource,
}

impl<T> Default for Field<T> {
    fn default() -> Self {
        Field {
            value: None,
            confidence: 0.0,
            source: FieldSource::Missing,
        }
    }
}

impl<T> Field<T> {
    pub fn new(value: T, source: FieldSource) -> Self {
        Field {
            value: Some(value),
            confidence: source.base_confidence(),
            source,
        }
    }

    /// A reading the extractor itself is unsure about - a regex that matched
    /// loosely, a vision model that hedged - scaled down from the source's
    /// usual confidence.
    pub fn with_confidence(value: T, source: FieldSource, confidence: f32) -> Self {
        Field {
            value: Some(value),
            confidence: confidence.clamp(0.0, 1.0),
            source,
        }
    }

    pub fn is_present(&self) -> bool {
        self.value.is_some()
    }

    pub fn needs_review(&self) -> bool {
        self.value.is_none() || self.confidence < REVIEW_THRESHOLD
    }

    pub fn as_ref(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// Takes `other` only if it is strictly better sourced. Ties keep `self`,
    /// so merge order never silently matters, and a Manual value can never be
    /// clobbered by a re-scan.
    pub fn merge_from(&mut self, other: Field<T>) {
        let better = other.value.is_some()
            && (self.value.is_none()
                || other.source > self.source
                || (other.source == self.source && other.confidence > self.confidence));
        if better {
            *self = other;
        }
    }
}

// ---------------------------------------------------------------------------
// Invoice kinds
// ---------------------------------------------------------------------------

/// The票种 that turn up in a Chinese expense report.
///
/// The first six are VAT invoices proper; the rest are the transport and
/// fixed-amount receipts that reimburse alongside them and have entirely
/// different layouts. Kept as one enum because the reimbursement sheet does
/// not care about the distinction - it just needs a date, a payee, and an
/// amount from each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InvoiceKind {
    /// 数电票（增值税专用发票）- fully digital, 20-digit number, no 发票代码.
    DigitalSpecial,
    /// 数电票（普通发票）
    DigitalGeneral,
    /// 增值税专用发票（纸质/电子，旧版）
    VatSpecial,
    /// 增值税普通发票
    VatGeneral,
    /// 增值税电子普通发票
    VatElectronicGeneral,
    /// 增值税普通发票（卷式）
    VatRoll,
    /// 铁路电子客票 / 火车票
    Train,
    /// 航空运输电子客票行程单
    AirItinerary,
    /// 出租车票 / 网约车电子发票以外的定额票
    Taxi,
    /// 定额发票
    FixedAmount,
    /// 过路过桥费
    Toll,
    /// 通用机打发票及其他未归类票据
    Other,
}

impl InvoiceKind {
    /// Recognises a 票种 from the title printed on an invoice, the value of an
    /// XML `InvoiceType` element, or the label a vision model reported.
    ///
    /// **All three extraction paths must go through this one function.** They
    /// each used to carry their own copy, and two of them had already drifted
    /// apart - which is not a hypothetical: the XML path classified every
    /// 数电票 as an 增值税电子普通发票, and since that older 票种 carries an
    /// 8-digit number, every fully-digital invoice in the ledger came out
    /// flagged 「发票号码位数异常」. One table, one precedence order.
    ///
    /// The precedence is the whole subtlety. `电子发票（普通发票）` is the
    /// title of a 数电票, and it *contains* both 电子 and 普通 - so a test for
    /// "电子 and 普通 means 增值税电子普通发票" matches it and is wrong. The
    /// parenthesised `电子发票（…）` form is the fully-digital marker, so it
    /// has to be tested before any of the substring rules below it.
    pub fn from_title(raw: &str) -> Option<InvoiceKind> {
        // Full-width parentheses on the invoice, ASCII in some XML. Folding
        // them means one entry per form instead of two.
        let title: String = raw
            .chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| match c {
                '（' => '(',
                '）' => ')',
                other => other,
            })
            .collect();

        // The 数电 platform's own numeric codes, as they appear in XML.
        match title.as_str() {
            "01" => return Some(InvoiceKind::VatSpecial),
            "04" => return Some(InvoiceKind::VatGeneral),
            "10" => return Some(InvoiceKind::VatElectronicGeneral),
            "31" => return Some(InvoiceKind::DigitalGeneral),
            "32" => return Some(InvoiceKind::DigitalSpecial),
            _ => {}
        }

        // Ordered longest-and-most-specific first; the first hit wins.
        const TABLE: &[(&str, InvoiceKind)] = &[
            // --- 数电票: the parenthesised form, and the words for it ---
            ("电子发票(增值税专用发票)", InvoiceKind::DigitalSpecial),
            ("电子发票(铁路电子客票)", InvoiceKind::Train),
            (
                "电子发票(航空运输电子客票行程单)",
                InvoiceKind::AirItinerary,
            ),
            ("电子发票(普通发票)", InvoiceKind::DigitalGeneral),
            ("数电票(专用)", InvoiceKind::DigitalSpecial),
            ("数电票(普通)", InvoiceKind::DigitalGeneral),
            ("全电发票(专用)", InvoiceKind::DigitalSpecial),
            ("全电发票(普通)", InvoiceKind::DigitalGeneral),
            // --- the older VAT family ---
            ("增值税电子专用发票", InvoiceKind::VatSpecial),
            ("增值税电子普通发票", InvoiceKind::VatElectronicGeneral),
            ("增值税专用发票", InvoiceKind::VatSpecial),
            ("增值税普通发票(卷式)", InvoiceKind::VatRoll),
            ("增值税普通发票", InvoiceKind::VatGeneral),
            // --- transport and fixed-amount receipts ---
            ("铁路电子客票", InvoiceKind::Train),
            ("火车票", InvoiceKind::Train),
            ("航空运输电子客票行程单", InvoiceKind::AirItinerary),
            ("行程单", InvoiceKind::AirItinerary),
            ("出租汽车", InvoiceKind::Taxi),
            ("出租车", InvoiceKind::Taxi),
            ("定额发票", InvoiceKind::FixedAmount),
            ("通行费", InvoiceKind::Toll),
        ];
        if let Some((_, kind)) = TABLE.iter().find(|(needle, _)| title.contains(needle)) {
            return Some(*kind);
        }

        // Last resort for platforms that write the words apart, e.g.
        // "数电票 增值税专用发票". Checked only after the table, so it can
        // never pre-empt a specific title.
        let digital = title.contains("数电") || title.contains("全电");
        if title.contains("专用") || title.contains("专票") {
            return Some(if digital {
                InvoiceKind::DigitalSpecial
            } else {
                InvoiceKind::VatSpecial
            });
        }
        if title.contains("普通") || title.contains("普票") {
            return Some(if digital {
                InvoiceKind::DigitalGeneral
            } else {
                InvoiceKind::VatGeneral
            });
        }
        None
    }

    pub fn label(self) -> &'static str {
        match self {
            InvoiceKind::DigitalSpecial => "数电票（专用）",
            InvoiceKind::DigitalGeneral => "数电票（普通）",
            InvoiceKind::VatSpecial => "增值税专用发票",
            InvoiceKind::VatGeneral => "增值税普通发票",
            InvoiceKind::VatElectronicGeneral => "增值税电子普通发票",
            InvoiceKind::VatRoll => "增值税普通发票（卷式）",
            InvoiceKind::Train => "铁路电子客票",
            InvoiceKind::AirItinerary => "航空运输电子客票行程单",
            InvoiceKind::Taxi => "出租车票",
            InvoiceKind::FixedAmount => "定额发票",
            InvoiceKind::Toll => "过路过桥费发票",
            InvoiceKind::Other => "其他票据",
        }
    }

    /// Whether this kind is a 数电票, whose 发票号码 is 20 digits and which
    /// carries no 发票代码 at all. Field validation keys off this.
    pub fn is_fully_digital(self) -> bool {
        matches!(
            self,
            InvoiceKind::DigitalSpecial | InvoiceKind::DigitalGeneral
        )
    }

    /// Whether input tax on this kind is deductible (进项税额可抵扣) - only
    /// 专用发票 are, which is the one tax fact a reimbursement sheet needs.
    pub fn is_deductible(self) -> bool {
        matches!(self, InvoiceKind::DigitalSpecial | InvoiceKind::VatSpecial)
    }
}

// ---------------------------------------------------------------------------
// Invoice
// ---------------------------------------------------------------------------

/// One line of an invoice's 货物或应税劳务、服务名称 table.
///
/// Only present when the source was structured (XML/OFD) or the text layer
/// was clean enough to split. The classifier reads `name` first - it is far
/// more specific than the seller's name, which is what makes rule-based
/// categorisation work at all.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceItem {
    /// 货物或应税劳务名称, e.g. "*住宿服务*住宿费".
    pub name: String,
    /// 规格型号
    pub spec: Option<String>,
    /// 单位
    pub unit: Option<String>,
    /// 数量 - a string because invoices write "1", "1.00" and "" alike and
    /// nothing downstream does arithmetic on it.
    pub quantity: Option<String>,
    pub unit_price: Option<Money>,
    pub amount: Option<Money>,
    /// 税率, as written: "6%", "免税", "不征税".
    pub tax_rate: Option<String>,
    pub tax: Option<Money>,
}

impl InvoiceItem {
    /// The 简称 inside the leading `*...*` that every VAT invoice line item
    /// starts with - "*住宿服务*住宿费" yields "住宿服务". This is the tax
    /// authority's own category code, so it is the single best signal the
    /// classifier has.
    pub fn tax_category(&self) -> Option<&str> {
        let rest = self.name.strip_prefix('*')?;
        let (category, _) = rest.split_once('*')?;
        let trimmed = category.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    }
}

/// A parsed invoice, before or after review.
///
/// Field order follows the invoice's own layout (identity, parties, money,
/// lines) so a reader comparing this against a real 发票 can follow along.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invoice {
    /// Row id once stored; None for a freshly extracted invoice.
    pub id: Option<i64>,

    pub kind: Option<InvoiceKind>,
    /// 发票号码 - 20 digits on 数电票, 8 on everything older.
    pub number: Field<String>,
    /// 发票代码 - 10 or 12 digits; absent by design on 数电票.
    pub code: Field<String>,
    /// 开票日期, normalised to `YYYY-MM-DD` regardless of how it was written.
    pub issued_on: Field<String>,
    /// 校验码 - only on 增值税电子普通发票, and only its last 6 digits are in
    /// the QR payload.
    pub check_code: Field<String>,

    pub buyer_name: Field<String>,
    pub buyer_tax_id: Field<String>,
    pub seller_name: Field<String>,
    pub seller_tax_id: Field<String>,

    /// 金额（不含税）
    pub amount_excl_tax: Field<Money>,
    /// 税额
    pub tax: Field<Money>,
    /// 价税合计 - the number that actually gets reimbursed.
    pub total: Field<Money>,

    pub items: Vec<InvoiceItem>,
    /// 备注
    pub remark: Field<String>,

    /// Assigned by the classifier or the user; see `classify`.
    pub category: Option<String>,
    /// Set when the category came from a rule, for "why this category?".
    pub category_rule: Option<String>,

    /// Absolute path of the file this came from.
    pub source_path: String,
    /// SHA-256 of the file bytes. Catches the same FILE imported twice even
    /// when extraction fails and there is no 发票号码 to dedupe on.
    pub file_hash: String,

    /// Cross-field problems found by `parse::validate`.
    pub issues: Vec<ValidationIssue>,
}

impl Invoice {
    /// Fields whose confidence is below the review bar, by display name.
    /// Drives the "N 项待复核" badge and the highlighted inputs in the detail
    /// pane - the UI never has to know which fields exist.
    pub fn fields_needing_review(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.number.needs_review() {
            out.push("发票号码");
        }
        if self.issued_on.needs_review() {
            out.push("开票日期");
        }
        if self.seller_name.needs_review() {
            out.push("销售方名称");
        }
        if self.total.needs_review() {
            out.push("价税合计");
        }
        out
    }

    /// The lowest confidence among the fields a reimbursement sheet cannot do
    /// without - a single number for sorting the list by "check these first".
    pub fn min_key_confidence(&self) -> f32 {
        [
            self.number.confidence,
            self.issued_on.confidence,
            self.total.confidence,
        ]
        .into_iter()
        .fold(1.0f32, f32::min)
    }
}

/// A cross-field inconsistency. Never blocks import - an invoice with a
/// problem still needs to be visible so the user can fix it - but it does
/// surface on the row and travels into the export as a warning column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "detail")]
pub enum ValidationIssue {
    /// 价税合计 ≠ 金额 + 税额. The single most useful check there is: it
    /// catches a misread digit in any of the three at once.
    TotalMismatch { expected: Money, found: Money },
    /// 发票号码 has the wrong number of digits for its 票种.
    NumberLength { found: usize, expected: usize },
    /// 开票日期 could not be normalised, or is not a real date.
    BadDate { raw: String },
    /// 开票日期 is in the future.
    FutureDate { date: String },
    /// A required field never came back from any extraction layer.
    MissingField { field: String },
}

impl ValidationIssue {
    pub fn message(&self) -> String {
        match self {
            ValidationIssue::TotalMismatch { expected, found } => {
                format!("价税合计对不上：金额+税额={expected}，票面={found}")
            }
            ValidationIssue::NumberLength { found, expected } => {
                format!("发票号码位数异常：{found} 位，应为 {expected} 位")
            }
            ValidationIssue::BadDate { raw } => format!("开票日期无法识别：{raw}"),
            ValidationIssue::FutureDate { date } => format!("开票日期晚于今天：{date}"),
            ValidationIssue::MissingField { field } => format!("缺少{field}"),
        }
    }

    /// Whether this is a hard error (red) or a caution (amber). A total that
    /// does not add up is a real defect; a missing 购买方税号 is a nag.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            ValidationIssue::TotalMismatch { .. } | ValidationIssue::BadDate { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_parses_the_forms_invoices_actually_use() {
        assert_eq!(Money::parse("1234.56"), Some(Money(123456)));
        assert_eq!(Money::parse("¥1234.56"), Some(Money(123456)));
        assert_eq!(Money::parse("￥1,234.56"), Some(Money(123456)));
        assert_eq!(Money::parse("（小写）1234.56"), Some(Money(123456)));
        assert_eq!(Money::parse("1234"), Some(Money(123400)));
        assert_eq!(Money::parse("0.5"), Some(Money(50)));
        assert_eq!(Money::parse("-88.00"), Some(Money(-8800)));
    }

    /// The refusal cases matter more than the successes: a wrong amount that
    /// looks parsed is worse than no amount at all.
    #[test]
    fn money_refuses_ambiguous_input() {
        assert_eq!(Money::parse(""), None);
        assert_eq!(Money::parse("价税合计"), None);
        // Two amounts run together by a greedy regex must not yield the first.
        assert_eq!(Money::parse("12.34 56.78"), None);
    }

    #[test]
    fn money_rounds_extra_precision_rather_than_truncating() {
        assert_eq!(Money::parse("1.005"), Some(Money(101)));
        assert_eq!(Money::parse("1.004"), Some(Money(100)));
    }

    #[test]
    fn decimal_strings_always_carry_two_places() {
        assert_eq!(Money(106_000).to_decimal_string(), "1060.00");
        assert_eq!(Money(43_50).to_decimal_string(), "43.50");
        assert_eq!(Money(5).to_decimal_string(), "0.05");
        assert_eq!(Money(0).to_decimal_string(), "0.00");
        assert_eq!(Money(-8_800).to_decimal_string(), "-88.00");
    }

    /// The guarantee behind `to_yuan_f64`: converting 分 to a spreadsheet
    /// double and back is lossless, so no cent can go missing in an export.
    /// Swept across the magnitudes an invoice actually occupies plus the
    /// awkward fractions that break naive float handling.
    #[test]
    fn every_cent_survives_the_xlsx_round_trip() {
        let mut cases: Vec<i64> = vec![0, 1, 5, 7, 10, 99, 100, 101, 105, 4_350];
        // Every 分 of the first hundred yuan.
        cases.extend(0..10_000);
        // Then decade by decade, up to a hundred million yuan.
        let mut magnitude = 10_000i64;
        while magnitude <= 10_000_000_000 {
            for offset in [0, 1, 7, 33, 99] {
                cases.push(magnitude + offset);
                cases.push(-(magnitude + offset));
            }
            magnitude *= 10;
        }

        for cents in cases {
            let round_tripped = (Money(cents).to_yuan_f64() * 100.0).round() as i64;
            assert_eq!(round_tripped, cents, "{cents} 分在浮点往返中丢失");
        }
    }

    /// Totals must be accumulated in integers, never in floats - that is
    /// where error compounds. A thousand additions of ¥0.07 is exactly
    /// ¥70.00, which the equivalent f64 sum is not.
    #[test]
    fn summing_money_is_exact() {
        let total: Money = std::iter::repeat_n(Money(7), 1_000).sum();
        assert_eq!(total, Money(7_000));
        assert_eq!(total.to_decimal_string(), "70.00");

        let float_sum: f64 = std::iter::repeat_n(0.07f64, 1_000).sum();
        assert_ne!(float_sum, 70.0, "浮点累加本来就会漂移，这正是不用它的原因");
    }

    #[test]
    fn money_displays_without_thousands_separators() {
        assert_eq!(Money(123456).to_string(), "¥1234.56");
        assert_eq!(Money(5).to_string(), "¥0.05");
        assert_eq!(Money(-8800).to_string(), "-¥88.00");
    }

    #[test]
    fn merge_keeps_the_better_source() {
        let mut f = Field::new("111".to_string(), FieldSource::Vision);
        f.merge_from(Field::new("222".to_string(), FieldSource::QrCode));
        assert_eq!(f.value.as_deref(), Some("222"));

        // ...and does not let a worse one back in.
        f.merge_from(Field::new("333".to_string(), FieldSource::Vision));
        assert_eq!(f.value.as_deref(), Some("222"));
    }

    #[test]
    fn manual_edits_survive_a_rescan() {
        let mut f = Field::new("corrected".to_string(), FieldSource::Manual);
        f.merge_from(Field::new("rescanned".to_string(), FieldSource::Xml));
        assert_eq!(f.value.as_deref(), Some("corrected"));
    }

    /// The bug this function exists to prevent, pinned: `电子发票（普通发票）`
    /// is a 数电票 (20-digit number), NOT an 增值税电子普通发票 (8 digits).
    /// Reading it as the latter made `validate` flag every fully-digital
    /// invoice in the ledger as having a malformed number.
    #[test]
    fn the_parenthesised_title_is_a_fully_digital_invoice() {
        for title in [
            "电子发票（普通发票）",
            "电子发票(普通发票)",
            "电子发票 （普通发票）",
        ] {
            assert_eq!(
                InvoiceKind::from_title(title),
                Some(InvoiceKind::DigitalGeneral),
                "{title}"
            );
        }
        assert_eq!(
            InvoiceKind::from_title("电子发票（增值税专用发票）"),
            Some(InvoiceKind::DigitalSpecial)
        );
    }

    /// ...while the older 票种 that genuinely is 8-digit still reads as itself.
    #[test]
    fn the_legacy_electronic_general_invoice_is_unchanged() {
        assert_eq!(
            InvoiceKind::from_title("增值税电子普通发票"),
            Some(InvoiceKind::VatElectronicGeneral)
        );
        assert_eq!(
            InvoiceKind::from_title("增值税专用发票"),
            Some(InvoiceKind::VatSpecial)
        );
        assert_eq!(
            InvoiceKind::from_title("增值税普通发票（卷式）"),
            Some(InvoiceKind::VatRoll)
        );
    }

    #[test]
    fn numeric_codes_and_transport_titles_resolve() {
        assert_eq!(
            InvoiceKind::from_title("31"),
            Some(InvoiceKind::DigitalGeneral)
        );
        assert_eq!(
            InvoiceKind::from_title("10"),
            Some(InvoiceKind::VatElectronicGeneral)
        );
        assert_eq!(
            InvoiceKind::from_title("铁路电子客票"),
            Some(InvoiceKind::Train)
        );
        assert_eq!(
            InvoiceKind::from_title("航空运输电子客票行程单"),
            Some(InvoiceKind::AirItinerary)
        );
        assert_eq!(
            InvoiceKind::from_title("通行费发票"),
            Some(InvoiceKind::Toll)
        );
    }

    /// A title found inside a whole page of invoice text still resolves - the
    /// text-layer path passes the entire compacted document in.
    #[test]
    fn a_title_embedded_in_a_page_of_text_is_found() {
        let page = "电子发票(普通发票)发票号码:24312000000012345678开票日期:2024年03月01日";
        assert_eq!(
            InvoiceKind::from_title(page),
            Some(InvoiceKind::DigitalGeneral)
        );
    }

    #[test]
    fn an_unrecognised_title_is_none_rather_than_a_guess() {
        assert_eq!(InvoiceKind::from_title(""), None);
        assert_eq!(InvoiceKind::from_title("收据"), None);
    }

    #[test]
    fn tax_category_reads_the_starred_prefix() {
        let item = InvoiceItem {
            name: "*住宿服务*住宿费".to_string(),
            ..Default::default()
        };
        assert_eq!(item.tax_category(), Some("住宿服务"));

        let plain = InvoiceItem {
            name: "住宿费".to_string(),
            ..Default::default()
        };
        assert_eq!(plain.tax_category(), None);
    }
}
