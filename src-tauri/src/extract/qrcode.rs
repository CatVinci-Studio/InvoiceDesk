//! The QR code every Chinese VAT invoice carries in its top-left corner.
//!
//! This is the most valuable extraction path for photographed invoices, and
//! the reason is worth stating plainly: **OCR degrades into confident wrong
//! answers, a QR code does not.** A crooked, shadowed phone photo will make
//! an OCR engine report `8` as `3` with no indication anything went wrong -
//! and a wrong 发票号码 defeats duplicate detection while a wrong amount
//! corrupts the reimbursement total. A QR code has error correction and a
//! checksum: it either decodes to exactly what was encoded, or it fails to
//! decode. There is no middle state to be wrong in.
//!
//! ## Payload format
//!
//! The classic 增值税发票 payload is a comma-separated list:
//!
//! ```text
//! 01,10,3100152130,12345678,1000.00,20230815,12345678901234567890,A1B2,
//! │  │  │          │        │       │        │                    │
//! │  │  │          │        │       │        │                    └ 校验码后 6 位（部分票种为空）
//! │  │  │          │        │       │        └ 校验码 / 密文
//! │  │  │          │        │       └ 开票日期 YYYYMMDD
//! │  │  │          │        └ 金额（专票为不含税金额，普票为价税合计）
//! │  │  │          └ 发票号码（8 位）
//! │  │  └ 发票代码（10 或 12 位）
//! │  └ 票种：01 专票 · 04 普票 · 10 电子普票 · 11 卷式 · 14 通行费 · 22 出租车…
//! └ 二维码版本，恒为 01
//! ```
//!
//! 数电票 (fully digital invoices) changed this: their 发票号码 is 20 digits,
//! there is no 发票代码 at all, and platforms have shipped several payload
//! shapes - some still comma-separated, some a verification URL with the
//! number in a query parameter. [`parse_payload`] handles the comma form
//! positionally and the URL form by field name, and refuses anything it
//! cannot place rather than guessing.
//!
//! ## What this deliberately does not do
//!
//! The QR payload is not a whole invoice: it has no 销售方名称, no 购买方, no
//! line items. It anchors the identity and the amount; the rest comes from
//! the text layer or the vision model. That is exactly the layering the
//! pipeline in `super` is built around.

use crate::model::{Field, FieldSource, InvoiceKind, Money};
use image::DynamicImage;

/// What a decoded invoice QR code yields. Every field optional: payload
/// shapes vary by 票种 and by platform, and a missing field simply means the
/// next layer has to supply it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct QrInvoice {
    pub kind: Option<InvoiceKind>,
    pub code: Option<String>,
    pub number: Option<String>,
    /// Whatever the payload's amount field held. Which amount this IS depends
    /// on 票种 - see [`amount_is_total`].
    pub amount: Option<Money>,
    /// Normalised to `YYYY-MM-DD`.
    pub issued_on: Option<String>,
    pub check_code: Option<String>,
}

impl QrInvoice {
    /// Whether [`Self::amount`] is 价税合计 rather than 不含税金额.
    ///
    /// This distinction is the one real trap in the payload: on a 专用发票
    /// the amount field is the tax-exclusive figure, while on a 普通发票 it
    /// is the tax-inclusive total. Writing the专票 value into 价税合计 would
    /// under-report every reimbursement by the tax amount, so when the kind
    /// says tax-exclusive the amount is routed to `amount_excl_tax` instead.
    fn amount_is_total(&self) -> bool {
        !matches!(
            self.kind,
            Some(InvoiceKind::VatSpecial) | Some(InvoiceKind::DigitalSpecial)
        )
    }

    /// Folds this into an [`Invoice`](crate::model::Invoice)'s fields.
    pub fn apply(&self, invoice: &mut crate::model::Invoice) {
        const S: FieldSource = FieldSource::QrCode;

        if invoice.kind.is_none() {
            invoice.kind = self.kind;
        }
        if let Some(v) = &self.code {
            invoice.code.merge_from(Field::new(v.clone(), S));
        }
        if let Some(v) = &self.number {
            invoice.number.merge_from(Field::new(v.clone(), S));
        }
        if let Some(v) = &self.issued_on {
            invoice.issued_on.merge_from(Field::new(v.clone(), S));
        }
        if let Some(v) = &self.check_code {
            invoice.check_code.merge_from(Field::new(v.clone(), S));
        }
        if let Some(v) = self.amount {
            if self.amount_is_total() {
                invoice.total.merge_from(Field::new(v, S));
            } else {
                invoice.amount_excl_tax.merge_from(Field::new(v, S));
            }
        }
    }
}

/// Maps the payload's two-digit 票种 code to a kind.
///
/// Unknown codes return None rather than a guess: 票种 decides whether the
/// amount field means 价税合计 or 不含税金额, so guessing it wrong corrupts
/// money. An unrecognised code just means the text layer names the kind.
fn kind_from_code(code: &str) -> Option<InvoiceKind> {
    match code {
        "01" => Some(InvoiceKind::VatSpecial),
        "02" => Some(InvoiceKind::VatSpecial), // 货物运输业增值税专用发票
        "03" => Some(InvoiceKind::Other),      // 机动车销售统一发票
        "04" => Some(InvoiceKind::VatGeneral),
        "10" => Some(InvoiceKind::VatElectronicGeneral),
        "11" => Some(InvoiceKind::VatRoll),
        "14" => Some(InvoiceKind::Toll),
        "15" => Some(InvoiceKind::Other),      // 二手车销售统一发票
        "20" => Some(InvoiceKind::VatSpecial), // 增值税电子专用发票
        "22" => Some(InvoiceKind::Taxi),
        // 数电票: 31 普通, 32 专用 as issued by the 全电 platform.
        "31" => Some(InvoiceKind::DigitalGeneral),
        "32" => Some(InvoiceKind::DigitalSpecial),
        _ => None,
    }
}

/// `20230815` or `2023-08-15` or `2023年08月15日` → `2023-08-15`.
/// None if it is not eight digits' worth of a plausible date.
fn normalise_date(raw: &str) -> Option<String> {
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    if digits.len() != 8 {
        return None;
    }
    let (y, rest) = digits.split_at(4);
    let (m, d) = rest.split_at(2);
    let (year, month, day): (u32, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    // 发票 predate 2000 in theory and never in an expense report; the upper
    // bound is deliberately loose so the app does not expire.
    if !(2000..=2999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// True for a string of digits of exactly the given length - the shape both
/// 发票号码 (8 or 20) and 发票代码 (10 or 12) have to satisfy.
fn digits_of_len(s: &str, lengths: &[usize]) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) && lengths.contains(&s.len())
}

/// Parses a decoded QR payload into invoice fields.
///
/// Returns None when the payload is not an invoice QR at all (a WeChat
/// payment code, a logo's decorative QR, a URL to a company site) - being
/// picky here is what keeps junk out of the ledger.
pub fn parse_payload(payload: &str) -> Option<QrInvoice> {
    let payload = payload.trim();
    if payload.is_empty() {
        return None;
    }

    // 数电票 platforms increasingly encode a verification URL. Named
    // parameters, so read them by name.
    if payload.starts_with("http://") || payload.starts_with("https://") {
        return parse_url_payload(payload);
    }

    let parts: Vec<&str> = payload.split(',').collect();
    // 01,票种,代码,号码,金额,日期 is the minimum that carries anything useful.
    if parts.len() < 6 || parts[0] != "01" {
        return None;
    }

    let kind = kind_from_code(parts[1]);
    let code = digits_of_len(parts[2], &[10, 12]).then(|| parts[2].to_string());
    // 数电票 payloads leave 发票代码 empty and put the 20-digit number in the
    // 号码 slot, so both lengths are accepted here.
    let number = digits_of_len(parts[3], &[8, 20]).then(|| parts[3].to_string());
    let amount = Money::parse(parts[4]);
    let issued_on = normalise_date(parts[5]);
    let check_code = parts
        .get(6)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(str::to_string);

    // A payload that yielded neither an invoice number nor a date is not an
    // invoice QR - the `01,` prefix alone is too weak a signal to trust.
    if number.is_none() && issued_on.is_none() {
        return None;
    }

    Some(QrInvoice {
        kind,
        code,
        number,
        amount,
        issued_on,
        check_code,
    })
}

/// The verification-URL payload form, e.g.
/// `https://inv-veri.chinatax.gov.cn/index.html?fpdm=...&fphm=...&kprq=...&kjje=...`
fn parse_url_payload(url: &str) -> Option<QrInvoice> {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut out = QrInvoice::default();

    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match key.to_ascii_lowercase().as_str() {
            // 发票代码
            "fpdm" if digits_of_len(value, &[10, 12]) => out.code = Some(value.to_string()),
            // 发票号码
            "fphm" if digits_of_len(value, &[8, 20]) => out.number = Some(value.to_string()),
            // 开票日期
            "kprq" => out.issued_on = normalise_date(value),
            // 开具金额（不含税）
            "kjje" => out.amount = Money::parse(value),
            // 校验码
            "jym" if !value.is_empty() => out.check_code = Some(value.to_string()),
            _ => {}
        }
    }

    // A 20-digit number with no 代码 is the 数电票 signature.
    if out.code.is_none() {
        if let Some(number) = &out.number {
            if number.len() == 20 {
                out.kind = Some(InvoiceKind::DigitalGeneral);
            }
        }
    }

    out.number.is_some().then_some(out)
}

/// Decodes every QR code in an image and returns the first that parses as an
/// invoice.
///
/// "Every" matters: invoices are photographed alongside WeChat codes, company
/// logos, and payment stickers, and the invoice's own QR is not always the
/// first one found. Trying all of them and filtering by
/// [`parse_payload`] costs nothing and removes a whole class of failure.
pub fn decode(image: &DynamicImage) -> Option<QrInvoice> {
    for payload in decode_all(image) {
        if let Some(invoice) = parse_payload(&payload) {
            return Some(invoice);
        }
    }
    None
}

/// Every barcode payload rxing can find, best-effort.
///
/// Tries the whole image first, then - if that finds nothing - the top-left
/// quadrant at 2x scale. Chinese VAT invoices always print the QR in the
/// top-left, and a full-page phone photo often renders it too small for the
/// detector while the cropped, upscaled corner decodes cleanly.
fn decode_all(image: &DynamicImage) -> Vec<String> {
    let found = scan(image);
    if !found.is_empty() {
        return found;
    }

    let (w, h) = (image.width(), image.height());
    if w < 400 || h < 300 {
        return found;
    }
    let corner = image.crop_imm(0, 0, w / 2, h / 2);
    let upscaled = corner.resize(
        corner.width() * 2,
        corner.height() * 2,
        image::imageops::FilterType::CatmullRom,
    );
    scan(&upscaled)
}

fn scan(image: &DynamicImage) -> Vec<String> {
    // Greyscale, because that is what the binarizer wants anyway - handing
    // rxing the luma plane avoids it copying the colour image to make one.
    // `detect_multiple_*` already defaults TryHarder to true, which is the
    // right trade here: a phone photo is worth several extra milliseconds.
    let (width, height) = (image.width(), image.height());
    rxing::helpers::detect_multiple_in_luma(image.to_luma8().into_raw(), width, height)
        .map(|results| results.iter().map(|r| r.getText().to_string()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_classic_electronic_general_invoice() {
        let qr =
            parse_payload("01,10,3100152130,12345678,1000.00,20230815,98765432109876543210,A1B2,")
                .expect("recognised as an invoice QR");
        assert_eq!(qr.kind, Some(InvoiceKind::VatElectronicGeneral));
        assert_eq!(qr.code.as_deref(), Some("3100152130"));
        assert_eq!(qr.number.as_deref(), Some("12345678"));
        assert_eq!(qr.amount, Some(Money(100_000)));
        assert_eq!(qr.issued_on.as_deref(), Some("2023-08-15"));
    }

    /// The trap this module exists to avoid: on a 专用发票 the payload's
    /// amount is tax-EXCLUSIVE, so it must not land in 价税合计.
    #[test]
    fn special_invoice_amount_is_not_the_total() {
        let qr = parse_payload("01,01,3100152130,12345678,1000.00,20230815,,").unwrap();
        let mut invoice = crate::model::Invoice::default();
        qr.apply(&mut invoice);
        assert_eq!(invoice.amount_excl_tax.value, Some(Money(100_000)));
        assert_eq!(invoice.total.value, None, "专票金额不是价税合计");
    }

    #[test]
    fn general_invoice_amount_is_the_total() {
        let qr = parse_payload("01,04,3100152130,12345678,1060.00,20230815,,").unwrap();
        let mut invoice = crate::model::Invoice::default();
        qr.apply(&mut invoice);
        assert_eq!(invoice.total.value, Some(Money(106_000)));
    }

    #[test]
    fn accepts_the_twenty_digit_fully_digital_number() {
        let qr = parse_payload("01,31,,24312000000012345678,88.00,20240301,,").unwrap();
        assert_eq!(qr.number.as_deref(), Some("24312000000012345678"));
        assert_eq!(qr.code, None, "数电票没有发票代码");
        assert_eq!(qr.kind, Some(InvoiceKind::DigitalGeneral));
    }

    #[test]
    fn parses_a_verification_url_payload() {
        let qr = parse_payload(
            "https://inv-veri.chinatax.gov.cn/index.html?fphm=24312000000012345678&kprq=20240301&kjje=88.00",
        )
        .unwrap();
        assert_eq!(qr.number.as_deref(), Some("24312000000012345678"));
        assert_eq!(qr.issued_on.as_deref(), Some("2024-03-01"));
        assert_eq!(qr.kind, Some(InvoiceKind::DigitalGeneral));
    }

    /// Non-invoice QR codes share the photo with the invoice's own. Accepting
    /// one would put a garbage row in the ledger.
    #[test]
    fn rejects_qr_codes_that_are_not_invoices() {
        assert!(parse_payload("https://weixin.qq.com/r/abcdef").is_none());
        assert!(parse_payload("hello world").is_none());
        assert!(parse_payload("").is_none());
        // Right prefix, no usable content - too weak to accept.
        assert!(parse_payload("01,99,,,,,").is_none());
    }

    #[test]
    fn rejects_implausible_dates_rather_than_normalising_them() {
        assert_eq!(normalise_date("20231301"), None, "13 月");
        assert_eq!(normalise_date("2023081"), None, "位数不足");
        assert_eq!(normalise_date("19990101"), None, "早于 2000 年");
        assert_eq!(
            normalise_date("2023年08月15日").as_deref(),
            Some("2023-08-15")
        );
    }
}
