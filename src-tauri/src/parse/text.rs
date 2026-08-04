//! Field extraction from an invoice's flattened text - a PDF text layer or
//! an OFD's drawn text runs.
//!
//! ## Why everything is matched against whitespace-stripped text
//!
//! Invoice layouts justify their labels by inserting spaces INSIDE the
//! words: a form prints `购 买 方`, `名  称`, `销 售 方`, and the text layer
//! faithfully reproduces every one of those gaps. A PDF extractor adds more
//! of its own at every positioned-run boundary. Matching `名称` against that
//! fails on most real invoices.
//!
//! So all matching happens on a copy with **every** whitespace character
//! removed. This is safe for exactly the fields being read here - Chinese
//! company names, tax IDs, invoice numbers and amounts contain no spaces -
//! and it fixes 校验码 for free, which is printed in space-separated groups
//! of five. Line items, which genuinely can contain spaces, come from the
//! structured layers instead.
//!
//! ## Why values terminate on the next known label
//!
//! Stripping whitespace welds a value to whatever label follows it:
//!
//! ```text
//! 名称：猫芬奇工作室统一社会信用代码/纳税人识别号：91310115MA1K3XYZ2B
//! ```
//!
//! A lazy `(.+?)` therefore has to stop at the next label rather than at a
//! delimiter, because there is no delimiter left. [`LABEL_BOUNDARY`] is that
//! stop list, and it is the main thing to extend when a new layout appears.

use crate::model::{Field, FieldSource, Invoice, InvoiceKind, Money};
use regex::Regex;
use std::sync::LazyLock;

/// Everything that can follow a field value, and therefore ends it.
const LABEL_BOUNDARY: &str = "统一社会信用代码|纳税人识别号|识别号|名称|地址电话|地址|电话|开户行及账号|开户行|账号|销售方|购买方|项目名称|货物或应税劳务|规格型号|合计|价税合计|备注|开票人|收款人|复核|销售方信息|购买方信息";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("field pattern compiles")
}

/// Whitespace-free copy of the text, plus full-width punctuation folded to
/// ASCII so one pattern covers `：` and `:` alike.
fn compact(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '：' => ':',
            '（' => '(',
            '）' => ')',
            '，' => ',',
            '￥' => '¥',
            other => other,
        })
        .collect()
}

// --- Identity ------------------------------------------------------------

/// These capture the WHOLE digit run and check its length afterwards, rather
/// than asking the pattern for a fixed count.
///
/// The reason is a trap worth naming: `(\d{8}|\d{20})` looks like it accepts
/// both lengths, but regex alternation is ordered and greedy-per-branch, so
/// against a 20-digit 数电票 number the first branch matches and the capture
/// is the first EIGHT digits. That is a silently truncated invoice number -
/// it looks plausible, it defeats duplicate detection, and nothing flags it.
/// Capturing `\d+` and validating in code cannot fail that way, and it also
/// lets a wrong-length number be REPORTED (see `validate`) instead of
/// quietly trimmed to fit.
static NUMBER: LazyLock<Regex> = LazyLock::new(|| re(r"发票号码:?(\d+)"));
static CODE: LazyLock<Regex> = LazyLock::new(|| re(r"发票代码:?(\d+)"));
static CHECK_CODE: LazyLock<Regex> = LazyLock::new(|| re(r"校验码:?(\d+)"));

/// A digit run of one of the allowed lengths, or None.
fn digits_of_len(value: &str, lengths: &[usize]) -> bool {
    lengths.contains(&value.len())
}
static DATE_CN: LazyLock<Regex> =
    LazyLock::new(|| re(r"开票日期:?(\d{4})年(\d{1,2})月(\d{1,2})日"));
static DATE_ISO: LazyLock<Regex> =
    LazyLock::new(|| re(r"开票日期:?(\d{4})[-/](\d{1,2})[-/](\d{1,2})"));

// --- Parties -------------------------------------------------------------

/// The colon is REQUIRED, and that is load-bearing: the goods table's header
/// reads `项目名称规格型号单位...` once whitespace is gone, which contains
/// `名称` and would otherwise match here and capture a slice of the table.
/// Every invoice form prints `名 称：` with its colon, so requiring it costs
/// nothing and removes the false positive outright.
static NAME: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"名称:(.+?)(?:{LABEL_BOUNDARY}|$)")));
static TAX_ID: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?:统一社会信用代码/?纳税人识别号|纳税人识别号|识别号):?([0-9A-Z]{15,20})")
});

// --- Money ---------------------------------------------------------------

/// `价税合计（大写）壹仟零陆拾圆整（小写）¥1060.00` - the parenthesised
/// 小写 marker is the most reliable anchor on the whole invoice, because the
/// 大写 form before it can never be confused for a number.
static TOTAL_LOWERCASE: LazyLock<Regex> =
    LazyLock::new(|| re(r"\(小写\)¥?(\d+(?:,\d{3})*\.\d{2})"));
/// Fallback for layouts with no 大写/小写 pair: the first amount within a
/// short reach of the 价税合计 label. Deliberately narrow (`{0,20}`) so it
/// cannot wander into the next row of the table.
static TOTAL_NEARBY: LazyLock<Regex> =
    LazyLock::new(|| re(r"价税合计[^\d]{0,20}¥?(\d+(?:,\d{3})*\.\d{2})"));
/// The 合计 row carries 金额 and 税额 side by side, both ¥-prefixed.
static SUBTOTAL_ROW: LazyLock<Regex> =
    LazyLock::new(|| re(r"合计¥(\d+(?:,\d{3})*\.\d{2})¥(\d+(?:,\d{3})*\.\d{2})"));

/// Byte range of one party's block within the compacted text.
type PartyRange = Option<(usize, usize)>;

/// Which party section a position in the text falls in.
///
/// Both parties have a 名称 and a 纳税人识别号, so the labels alone cannot
/// tell them apart - only position relative to the 购买方 / 销售方 markers
/// can. Getting this backwards swaps who bought from whom on every invoice,
/// so it is worth the explicit scan.
fn party_ranges(compact: &str) -> (PartyRange, PartyRange) {
    let buyer_start = ["购买方信息", "购买方"]
        .iter()
        .find_map(|m| compact.find(m));
    let seller_start = ["销售方信息", "销售方"]
        .iter()
        .find_map(|m| compact.find(m));

    match (buyer_start, seller_start) {
        // The usual layout: 购买方 block, then 销售方 block, then the goods
        // table. Each section runs until the next one starts.
        (Some(b), Some(s)) if b < s => (Some((b, s)), Some((s, compact.len()))),
        // Some 数电票 layouts print 销售方 first.
        (Some(b), Some(s)) => (Some((b, compact.len())), Some((s, b))),
        (Some(b), None) => (Some((b, compact.len())), None),
        (None, Some(s)) => (None, Some((s, compact.len()))),
        (None, None) => (None, None),
    }
}

/// First capture of `pattern` within `compact[range]`.
fn find_in(compact: &str, range: PartyRange, pattern: &Regex) -> Option<String> {
    let (start, end) = range?;
    // Byte offsets came from `find` on this same string, so they are on char
    // boundaries; the guard is against a future edit that computes them
    // differently rather than against today's code.
    let slice = compact.get(start..end)?;
    let value = pattern.captures(slice)?.get(1)?.as_str().trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// 票种, read from the title line every invoice carries.
///
/// The precedence problem this used to solve locally now lives on
/// `InvoiceKind::from_title`, shared with the XML and vision paths - see its
/// doc comment for why three copies was a bug waiting to happen.
fn detect_kind(compact: &str) -> Option<InvoiceKind> {
    // The compacted text is the WHOLE invoice, not just its title, so this
    // scans for the first title-like substring rather than matching the
    // string as a whole. Titles are printed first, so the first hit is it.
    InvoiceKind::from_title(compact)
}

/// Extracts what it can from flattened invoice text.
///
/// Never fails: an unrecognisable blob yields an invoice with every field
/// empty, which is exactly what the pipeline needs in order to fall through
/// to the next layer. `source` distinguishes a PDF text layer from OFD drawn
/// text for provenance display; both are equally trustworthy.
pub fn parse(text: &str, source: FieldSource) -> Invoice {
    let c = compact(text);
    let (buyer_range, seller_range) = party_ranges(&c);

    let text_field = |value: Option<String>| -> Field<String> {
        value.map(|v| Field::new(v, source)).unwrap_or_default()
    };

    let capture = |pattern: &Regex| -> Option<String> {
        pattern
            .captures(&c)
            .and_then(|m| m.get(1))
            .map(|m| m.as_str().to_string())
    };

    let issued_on = DATE_CN
        .captures(&c)
        .or_else(|| DATE_ISO.captures(&c))
        .and_then(|m| {
            let year: u32 = m.get(1)?.as_str().parse().ok()?;
            let month: u32 = m.get(2)?.as_str().parse().ok()?;
            let day: u32 = m.get(3)?.as_str().parse().ok()?;
            ((1..=12).contains(&month) && (1..=31).contains(&day))
                .then(|| format!("{year:04}-{month:02}-{day:02}"))
        });

    let total = capture(&TOTAL_LOWERCASE)
        .and_then(|v| Money::parse(&v))
        .map(|v| Field::new(v, source))
        // The fallback anchor is looser, so it is recorded as such rather
        // than claiming the same confidence as the 小写 form.
        .or_else(|| {
            capture(&TOTAL_NEARBY)
                .and_then(|v| Money::parse(&v))
                .map(|v| Field::with_confidence(v, source, 0.80))
        })
        .unwrap_or_default();

    let (amount_excl_tax, tax) = SUBTOTAL_ROW
        .captures(&c)
        .map(|m| {
            let amount = m.get(1).and_then(|g| Money::parse(g.as_str()));
            let tax = m.get(2).and_then(|g| Money::parse(g.as_str()));
            (
                amount.map(|v| Field::new(v, source)).unwrap_or_default(),
                tax.map(|v| Field::new(v, source)).unwrap_or_default(),
            )
        })
        .unwrap_or_default();

    // With no 购买方/销售方 markers, fall back to ordinal position: the
    // buyer's block is printed first on every layout in circulation.
    let (buyer_name, seller_name) = if buyer_range.is_some() || seller_range.is_some() {
        (
            find_in(&c, buyer_range, &NAME),
            find_in(&c, seller_range, &NAME),
        )
    } else {
        let mut names = NAME
            .captures_iter(&c)
            .filter_map(|m| m.get(1).map(|g| g.as_str().trim().to_string()))
            .filter(|s| !s.is_empty());
        (names.next(), names.next())
    };

    let (buyer_tax_id, seller_tax_id) = if buyer_range.is_some() || seller_range.is_some() {
        (
            find_in(&c, buyer_range, &TAX_ID),
            find_in(&c, seller_range, &TAX_ID),
        )
    } else {
        let mut ids = TAX_ID
            .captures_iter(&c)
            .filter_map(|m| m.get(1).map(|g| g.as_str().to_string()));
        (ids.next(), ids.next())
    };

    // Length is validated rather than baked into the pattern - see NUMBER.
    let number = capture(&NUMBER).filter(|v| digits_of_len(v, &[8, 20]));
    let code = capture(&CODE).filter(|v| digits_of_len(v, &[10, 12]));
    let check_code = capture(&CHECK_CODE).filter(|v| digits_of_len(v, &[20]));

    Invoice {
        kind: detect_kind(&c),
        number: text_field(number),
        code: text_field(code),
        issued_on: text_field(issued_on),
        check_code: text_field(check_code),
        buyer_name: text_field(buyer_name),
        buyer_tax_id: text_field(buyer_tax_id),
        seller_name: text_field(seller_name),
        seller_tax_id: text_field(seller_tax_id),
        amount_excl_tax,
        tax,
        total,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 数电票 text layer, with the justification spaces real invoices have.
    const DIGITAL: &str = "电子发票（普通发票）
        发 票 号 码：24312000000012345678
        开 票 日 期：2024年03月01日
        购买方信息  名  称：猫芬奇工作室
        统一社会信用代码/纳税人识别号：91310115MA1K3XYZ2B
        销售方信息  名  称：杭州云栖酒店管理有限公司
        统一社会信用代码/纳税人识别号：91330106MA2GX8QP1A
        项目名称 规格型号 单位 数量 单价 金额 税率/征收率 税额
        *住宿服务*住宿费   2  500.00  1000.00  6%  60.00
        合  计 ¥1000.00 ¥60.00
        价税合计（大写）壹仟零陆拾圆整 （小写）¥1060.00";

    #[test]
    fn reads_a_fully_digital_invoice_through_its_justification_spaces() {
        let inv = parse(DIGITAL, FieldSource::PdfText);
        assert_eq!(inv.number.value.as_deref(), Some("24312000000012345678"));
        assert_eq!(inv.issued_on.value.as_deref(), Some("2024-03-01"));
        assert_eq!(inv.total.value, Some(Money(106_000)));
        assert_eq!(inv.amount_excl_tax.value, Some(Money(100_000)));
        assert_eq!(inv.tax.value, Some(Money(6_000)));
        assert_eq!(inv.kind, Some(InvoiceKind::DigitalGeneral));
    }

    /// The failure that would corrupt every invoice silently: swapping who
    /// bought from whom.
    #[test]
    fn buyer_and_seller_are_not_swapped() {
        let inv = parse(DIGITAL, FieldSource::PdfText);
        assert_eq!(inv.buyer_name.value.as_deref(), Some("猫芬奇工作室"));
        assert_eq!(
            inv.seller_name.value.as_deref(),
            Some("杭州云栖酒店管理有限公司")
        );
        assert_eq!(
            inv.buyer_tax_id.value.as_deref(),
            Some("91310115MA1K3XYZ2B")
        );
        assert_eq!(
            inv.seller_tax_id.value.as_deref(),
            Some("91330106MA2GX8QP1A")
        );
    }

    #[test]
    fn handles_the_layout_that_prints_seller_first() {
        let text = "电子发票（普通发票）发票号码：24312000000012345678
            销售方信息 名称：某某科技有限公司 纳税人识别号：91330106MA2GX8QP1A
            购买方信息 名称：猫芬奇工作室 纳税人识别号：91310115MA1K3XYZ2B";
        let inv = parse(text, FieldSource::PdfText);
        assert_eq!(inv.seller_name.value.as_deref(), Some("某某科技有限公司"));
        assert_eq!(inv.buyer_name.value.as_deref(), Some("猫芬奇工作室"));
    }

    /// The older paper/electronic VAT invoice, which has a 发票代码 and a
    /// 校验码 printed in groups of five.
    #[test]
    fn reads_a_legacy_vat_electronic_invoice() {
        let text = "增值税电子普通发票
            发票代码：3100152130  发票号码：12345678
            开票日期：2023年08月15日
            校 验 码：12345 67890 12345 67890
            购买方 名称：猫芬奇工作室 纳税人识别号：91310115MA1K3XYZ2B
            销售方 名称：上海某某餐饮管理有限公司 纳税人识别号：913101150000000000
            价税合计（大写）壹佰零陆圆整（小写）¥106.00";
        let inv = parse(text, FieldSource::PdfText);
        assert_eq!(inv.code.value.as_deref(), Some("3100152130"));
        assert_eq!(inv.number.value.as_deref(), Some("12345678"));
        assert_eq!(
            inv.check_code.value.as_deref(),
            Some("12345678901234567890")
        );
        assert_eq!(inv.total.value, Some(Money(10_600)));
        assert_eq!(inv.kind, Some(InvoiceKind::VatElectronicGeneral));
    }

    /// 数电专票 also contains the substring 增值税专用发票 - filing it as a
    /// paper 专票 would put the wrong 票种 on every one of them.
    #[test]
    fn digital_special_is_not_filed_as_paper_special() {
        let inv = parse(
            "电子发票（增值税专用发票）发票号码：24312000000012345678",
            FieldSource::PdfText,
        );
        assert_eq!(inv.kind, Some(InvoiceKind::DigitalSpecial));
    }

    #[test]
    fn thousands_separators_in_the_total_survive() {
        let inv = parse("价税合计（小写）¥12,345.67", FieldSource::PdfText);
        assert_eq!(inv.total.value, Some(Money(1_234_567)));
    }

    /// The fallback anchor is looser than the 小写 one and must say so, or
    /// the review badge never fires on the layouts that need it most.
    #[test]
    fn the_fallback_total_anchor_reports_lower_confidence() {
        let inv = parse("价税合计 ¥1060.00", FieldSource::PdfText);
        assert_eq!(inv.total.value, Some(Money(106_000)));
        assert!(inv.total.confidence < 0.90);
        assert!(inv.total.needs_review());
    }

    #[test]
    fn unrecognisable_text_yields_empty_fields_rather_than_wrong_ones() {
        let inv = parse("这是一份合同，不是发票。", FieldSource::PdfText);
        assert!(inv.number.value.is_none());
        assert!(inv.total.value.is_none());
        assert!(inv.number.needs_review());
    }

    /// A nine-digit run must not be accepted as an eight-digit 发票号码 with
    /// a stray digit attached.
    #[test]
    fn invoice_number_length_is_exact() {
        let inv = parse("发票号码：123456789", FieldSource::PdfText);
        assert_eq!(inv.number.value, None);
    }

    /// Regression guard for the alternation trap described on NUMBER: a
    /// 20-digit 数电票 number must come back whole, not as its first 8
    /// digits. A truncated number is the worst kind of wrong - plausible,
    /// unflagged, and it silently breaks duplicate detection.
    #[test]
    fn twenty_digit_numbers_are_never_truncated_to_eight() {
        let inv = parse("发票号码：24312000000012345678", FieldSource::PdfText);
        assert_eq!(inv.number.value.as_deref(), Some("24312000000012345678"));
    }

    /// The goods table header contains 名称; it must not be read as a party
    /// name when the invoice has no 购买方/销售方 markers at all.
    #[test]
    fn the_goods_table_header_is_not_mistaken_for_a_party_name() {
        let inv = parse(
            "发票号码：12345678 项目名称 规格型号 单位 数量 单价 金额 税率 税额",
            FieldSource::PdfText,
        );
        assert_eq!(inv.buyer_name.value, None);
        assert_eq!(inv.seller_name.value, None);
    }
}
