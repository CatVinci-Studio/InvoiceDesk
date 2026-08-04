//! The authoritative source: 电子发票 XML.
//!
//! When it is present this beats every other layer outright, because it is
//! not a *reading* of the invoice - it IS the invoice, as the tax system
//! issued it. It arrives two ways, and both land here:
//!
//! - attached inside a 数电票 PDF's `EmbeddedFiles` tree (see [`super::pdf`])
//! - as an XML part inside an OFD package (see [`super::ofd`])
//!
//! ## Why this parser is tag-driven rather than path-driven
//!
//! There is no single XML schema in the field. The 全电发票 standard, the
//! older 增值税电子发票 format, and several provincial platforms' variants
//! all disagree about element nesting, namespace prefixes, and casing - and
//! they keep changing. A parser written against one document tree returns
//! *nothing at all* against the next one, which is the worst possible
//! failure mode for the layer that is supposed to be exact.
//!
//! So this walks the whole document and indexes leaf text by LOCAL element
//! name, then resolves each invoice field against a list of known aliases.
//! Nesting, prefixes and ordering stop mattering; only the tag vocabulary
//! does, and that is the part the variants actually share. Adding support
//! for a new platform's dialect means adding a string to an alias list.

use crate::model::{Field, FieldSource, Invoice, InvoiceItem, InvoiceKind, Money};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;

/// Every leaf's text, indexed by lowercased local tag name. Values are kept
/// in document order; the first non-empty one wins, which matches how these
/// documents put summary fields ahead of per-line repeats.
#[derive(Debug, Default)]
struct TagIndex {
    leaves: HashMap<String, Vec<String>>,
    /// Each repeated item container's own child leaves, in order.
    item_groups: Vec<HashMap<String, String>>,
}

impl TagIndex {
    fn first(&self, aliases: &[&str]) -> Option<&str> {
        for alias in aliases {
            if let Some(values) = self.leaves.get(&normalise(alias)) {
                if let Some(v) = values.iter().find(|v| !v.trim().is_empty()) {
                    return Some(v.trim());
                }
            }
        }
        None
    }
}

/// Lowercase, and drop the separators that differ between dialects: the
/// standard's own `TotalTax-includedAmount` has a hyphen, some platforms
/// write `total_tax_included_amount`, others `TotalTaxIncludedAmount`.
fn normalise(tag: &str) -> String {
    tag.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Strips any namespace prefix: `<ns2:InvoiceNumber>` → `invoicenumber`.
fn local_name(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let local = text.rsplit(':').next().unwrap_or(&text);
    normalise(local)
}

/// Element names that mean "one line of the goods table". Their children are
/// collected as a group rather than flattened into the document-wide index,
/// so a five-line invoice yields five items instead of one field with five
/// values stacked in it.
const ITEM_CONTAINERS: &[&str] = &[
    "issuiteminformation",
    "invoiceitem",
    "itemlist",
    "item",
    "goodsinfo",
    "goods",
    "detail",
    "detaillist",
    "commodity",
];

fn is_item_container(tag: &str) -> bool {
    ITEM_CONTAINERS.contains(&tag)
}

/// Indexes a document. Malformed XML yields whatever was read before the
/// error rather than nothing - a truncated attachment still usually carries
/// the header fields, and half an invoice beats none.
fn index(xml: &str) -> TagIndex {
    let mut reader = Reader::from_str(xml);
    let config = reader.config_mut();
    config.trim_text(true);
    config.check_end_names = false;

    let mut out = TagIndex::default();
    let mut stack: Vec<String> = Vec::new();
    // Depth at which the currently-open item container started, plus the
    // group being filled. Nested containers keep the outermost one, which is
    // the row - inner ones tend to be sub-details.
    let mut item_depth: Option<usize> = None;
    let mut current_item: HashMap<String, String> = HashMap::new();
    let mut pending_text: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let tag = local_name(e.name().as_ref());
                if item_depth.is_none() && is_item_container(&tag) {
                    item_depth = Some(stack.len());
                    current_item.clear();
                }
                stack.push(tag);
                pending_text = None;
            }
            Ok(Event::Text(e)) => {
                pending_text = e.decode().ok().map(|s| s.trim().to_string());
            }
            Ok(Event::CData(e)) => {
                pending_text = String::from_utf8(e.to_vec())
                    .ok()
                    .map(|s| s.trim().to_string());
            }
            Ok(Event::End(_)) => {
                let Some(tag) = stack.pop() else { continue };

                if let Some(text) = pending_text.take() {
                    if !text.is_empty() {
                        // Inside an item container the leaf belongs to that
                        // row; outside it, to the document.
                        if item_depth.is_some_and(|d| stack.len() > d) {
                            current_item.entry(tag.clone()).or_insert(text.clone());
                        }
                        out.leaves.entry(tag).or_default().push(text);
                    }
                }

                if item_depth == Some(stack.len()) {
                    item_depth = None;
                    if !current_item.is_empty() {
                        out.item_groups.push(std::mem::take(&mut current_item));
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    out
}

// --- Field aliases -------------------------------------------------------
//
// Ordered most-specific first: `InvoiceNumber` is unambiguous, while a bare
// `Number` could be anything, so it comes last and only matches when nothing
// better did.

const NUMBER: &[&str] = &["InvoiceNumber", "InvoiceNo", "Fphm", "InvoiceNum", "FPHM"];
const CODE: &[&str] = &["InvoiceCode", "Fpdm", "FPDM"];
const ISSUE_TIME: &[&str] = &["IssueTime", "InvoiceDate", "IssueDate", "Kprq", "KPRQ"];
const CHECK_CODE: &[&str] = &["CheckCode", "Jym", "InvoiceCheckCode"];
const INVOICE_TYPE: &[&str] = &["InvoiceType", "Fplx", "InvoiceTypeCode"];

const BUYER_NAME: &[&str] = &["BuyerName", "PurchaserName", "GfMc", "BuyerNam"];
const BUYER_TAX_ID: &[&str] = &[
    "BuyerTaxID",
    "PurchaserTaxID",
    "BuyerTaxNo",
    "GfNsrsbh",
    "BuyerTaxpayerID",
];
const SELLER_NAME: &[&str] = &["SellerName", "SalerName", "XfMc", "SellerNam"];
const SELLER_TAX_ID: &[&str] = &[
    "SellerTaxID",
    "SalerTaxID",
    "SellerTaxNo",
    "XfNsrsbh",
    "SellerTaxpayerID",
];

const AMOUNT_EXCL_TAX: &[&str] = &[
    "TotalAmWithoutTax",
    "TotalAmountWithoutTax",
    "AmountWithoutTax",
    "HjJe",
    "TotalAmount",
];
const TAX: &[&str] = &["TotalTaxAm", "TotalTaxAmount", "TaxAmount", "HjSe"];
const TOTAL: &[&str] = &[
    // The standard's own spelling, hyphen and all - normalise() strips it.
    "TotalTax-includedAmount",
    "TotalTaxIncludedAmount",
    "TotalAmWithTax",
    "AmountWithTax",
    "JshjJe",
    "TotalIncludingTax",
];
const REMARK: &[&str] = &["Remark", "Note", "Bz", "Memo"];

const ITEM_NAME: &[&str] = &["ItemName", "GoodsName", "Spmc", "Name"];
const ITEM_SPEC: &[&str] = &["SpecMod", "Specification", "Ggxh", "Spec"];
const ITEM_UNIT: &[&str] = &["MeasurementDimension", "Unit", "Dw"];
const ITEM_QUANTITY: &[&str] = &["Quantity", "Sl", "Num"];
const ITEM_UNIT_PRICE: &[&str] = &["UnPrice", "UnitPrice", "Dj", "Price"];
const ITEM_AMOUNT: &[&str] = &["Amount", "Je", "DetailAmount"];
const ITEM_TAX_RATE: &[&str] = &["TaxRateValue", "TaxRate", "Slv"];
const ITEM_TAX: &[&str] = &["ComTaxAm", "TaxAm", "Se", "TaxAmount"];

fn lookup(group: &HashMap<String, String>, aliases: &[&str]) -> Option<String> {
    aliases
        .iter()
        .find_map(|a| group.get(&normalise(a)))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `2024-03-01T10:22:33`, `2024-03-01`, `20240301`, `2024年03月01日`
/// → `2024-03-01`. None when there is no plausible date in there.
fn normalise_date(raw: &str) -> Option<String> {
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 8 {
        return None;
    }
    let (y, rest) = digits.split_at(4);
    let (m, rest) = rest.split_at(2);
    let d = &rest[..2];
    let (year, month, day): (u32, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    if !(2000..=2999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// Parses an invoice out of XML. Returns None only when the document holds
/// no invoice number at all - without that there is nothing to dedupe on and
/// the caller should fall through to the next extraction layer.
pub fn parse(xml: &str) -> Option<Invoice> {
    let idx = index(xml);
    let number = idx.first(NUMBER)?.to_string();

    const S: FieldSource = FieldSource::Xml;
    let text_field = |aliases: &[&str]| -> Field<String> {
        idx.first(aliases)
            .map(|v| Field::new(v.to_string(), S))
            .unwrap_or_default()
    };
    let money_field = |aliases: &[&str]| -> Field<Money> {
        idx.first(aliases)
            .and_then(Money::parse)
            .map(|v| Field::new(v, S))
            .unwrap_or_default()
    };

    let kind = idx
        .first(INVOICE_TYPE)
        .and_then(InvoiceKind::from_title)
        // No type element, but a 20-digit number and no 发票代码 is the
        // 数电票 signature and worth inferring.
        .or_else(|| {
            (number.len() == 20 && idx.first(CODE).is_none()).then_some(InvoiceKind::DigitalGeneral)
        });

    let items = idx
        .item_groups
        .iter()
        .filter_map(|group| {
            let name = lookup(group, ITEM_NAME)?;
            Some(InvoiceItem {
                name,
                spec: lookup(group, ITEM_SPEC),
                unit: lookup(group, ITEM_UNIT),
                quantity: lookup(group, ITEM_QUANTITY),
                unit_price: lookup(group, ITEM_UNIT_PRICE)
                    .as_deref()
                    .and_then(Money::parse),
                amount: lookup(group, ITEM_AMOUNT).as_deref().and_then(Money::parse),
                tax_rate: lookup(group, ITEM_TAX_RATE),
                tax: lookup(group, ITEM_TAX).as_deref().and_then(Money::parse),
            })
        })
        .collect();

    Some(Invoice {
        kind,
        number: Field::new(number, S),
        code: text_field(CODE),
        issued_on: idx
            .first(ISSUE_TIME)
            .and_then(normalise_date)
            .map(|v| Field::new(v, S))
            .unwrap_or_default(),
        check_code: text_field(CHECK_CODE),
        buyer_name: text_field(BUYER_NAME),
        buyer_tax_id: text_field(BUYER_TAX_ID),
        seller_name: text_field(SELLER_NAME),
        seller_tax_id: text_field(SELLER_TAX_ID),
        amount_excl_tax: money_field(AMOUNT_EXCL_TAX),
        tax: money_field(TAX),
        total: money_field(TOTAL),
        items,
        remark: text_field(REMARK),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped after the 全电发票 standard, namespace prefixes and all.
    const FULLY_DIGITAL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ns2:EInvoice xmlns:ns2="http://www.chinatax.gov.cn/tirs/">
  <ns2:Header><ns2:Ver>1.0</ns2:Ver></ns2:Header>
  <ns2:EInvoiceData>
    <ns2:SellerInformation>
      <ns2:SellerName>杭州云栖酒店管理有限公司</ns2:SellerName>
      <ns2:SellerTaxID>91330106MA2GX8QP1A</ns2:SellerTaxID>
    </ns2:SellerInformation>
    <ns2:BuyerInformation>
      <ns2:BuyerName>猫芬奇工作室</ns2:BuyerName>
      <ns2:BuyerTaxID>91310115MA1K3XYZ2B</ns2:BuyerTaxID>
    </ns2:BuyerInformation>
    <ns2:BasicInformation>
      <ns2:InvoiceNumber>24312000000012345678</ns2:InvoiceNumber>
      <ns2:IssueTime>2024-03-01T10:22:33</ns2:IssueTime>
      <ns2:TotalAmWithoutTax>1000.00</ns2:TotalAmWithoutTax>
      <ns2:TotalTaxAm>60.00</ns2:TotalTaxAm>
      <ns2:TotalTax-includedAmount>1060.00</ns2:TotalTax-includedAmount>
      <ns2:Remark>差旅住宿</ns2:Remark>
    </ns2:BasicInformation>
    <ns2:IssuItemInformation>
      <ns2:ItemName>*住宿服务*住宿费</ns2:ItemName>
      <ns2:Quantity>2</ns2:Quantity>
      <ns2:UnPrice>500.00</ns2:UnPrice>
      <ns2:Amount>1000.00</ns2:Amount>
      <ns2:TaxRateValue>0.06</ns2:TaxRateValue>
      <ns2:ComTaxAm>60.00</ns2:ComTaxAm>
    </ns2:IssuItemInformation>
  </ns2:EInvoiceData>
</ns2:EInvoice>"#;

    #[test]
    fn parses_the_fully_digital_standard() {
        let inv = parse(FULLY_DIGITAL).expect("has an invoice number");
        assert_eq!(inv.number.value.as_deref(), Some("24312000000012345678"));
        assert_eq!(inv.issued_on.value.as_deref(), Some("2024-03-01"));
        assert_eq!(
            inv.seller_name.value.as_deref(),
            Some("杭州云栖酒店管理有限公司")
        );
        assert_eq!(inv.buyer_name.value.as_deref(), Some("猫芬奇工作室"));
        assert_eq!(inv.amount_excl_tax.value, Some(Money(100_000)));
        assert_eq!(inv.tax.value, Some(Money(6_000)));
        assert_eq!(inv.total.value, Some(Money(106_000)));
        assert_eq!(inv.kind, Some(InvoiceKind::DigitalGeneral));
        // Everything from XML is authoritative.
        assert_eq!(inv.total.confidence, 1.0);
        assert_eq!(inv.total.source, FieldSource::Xml);
    }

    #[test]
    fn collects_line_items_as_rows_not_stacked_fields() {
        let inv = parse(FULLY_DIGITAL).unwrap();
        assert_eq!(inv.items.len(), 1);
        assert_eq!(inv.items[0].name, "*住宿服务*住宿费");
        assert_eq!(inv.items[0].amount, Some(Money(100_000)));
        assert_eq!(inv.items[0].tax_category(), Some("住宿服务"));
    }

    /// The point of the tag-driven design: a completely different nesting
    /// and vocabulary still parses.
    #[test]
    fn parses_a_dialect_with_different_nesting_and_tags() {
        let other = r#"<Invoice>
          <Fphm>12345678</Fphm>
          <Fpdm>3100152130</Fpdm>
          <Kprq>20230815</Kprq>
          <XfMc>上海某某科技有限公司</XfMc>
          <GfMc>猫芬奇工作室</GfMc>
          <HjJe>1000.00</HjJe>
          <HjSe>60.00</HjSe>
          <JshjJe>1060.00</JshjJe>
          <Detail><Spmc>*信息技术服务*技术服务费</Spmc><Je>1000.00</Je></Detail>
        </Invoice>"#;
        let inv = parse(other).unwrap();
        assert_eq!(inv.number.value.as_deref(), Some("12345678"));
        assert_eq!(inv.code.value.as_deref(), Some("3100152130"));
        assert_eq!(inv.issued_on.value.as_deref(), Some("2023-08-15"));
        assert_eq!(
            inv.seller_name.value.as_deref(),
            Some("上海某某科技有限公司")
        );
        assert_eq!(inv.total.value, Some(Money(106_000)));
        assert_eq!(inv.items.len(), 1);
    }

    #[test]
    fn multiple_line_items_stay_separate() {
        let xml = r#"<Invoice><InvoiceNumber>1</InvoiceNumber>
          <Item><ItemName>*餐饮服务*餐费</ItemName><Amount>100.00</Amount></Item>
          <Item><ItemName>*住宿服务*住宿费</ItemName><Amount>200.00</Amount></Item>
        </Invoice>"#;
        let inv = parse(xml).unwrap();
        assert_eq!(inv.items.len(), 2);
        assert_eq!(inv.items[0].amount, Some(Money(10_000)));
        assert_eq!(inv.items[1].amount, Some(Money(20_000)));
    }

    #[test]
    fn no_invoice_number_means_fall_through_to_the_next_layer() {
        assert!(parse("<Invoice><Foo>bar</Foo></Invoice>").is_none());
        assert!(parse("not xml at all").is_none());
    }

    /// A truncated attachment must still yield its header fields.
    #[test]
    fn malformed_xml_yields_what_was_readable() {
        let truncated = "<Invoice><InvoiceNumber>12345678</InvoiceNumber><XfMc>某公司</XfMc";
        let inv = parse(truncated).unwrap();
        assert_eq!(inv.number.value.as_deref(), Some("12345678"));
    }
}
