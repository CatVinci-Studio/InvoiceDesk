//! OFD (GB/T 33190) - China's own layout document format, and the other
//! half of what the tax platform hands out alongside PDF.
//!
//! An OFD is a zip. `OFD.xml` at the root points at a document, the document
//! points at pages, and each page's `Content.xml` holds the drawn text in
//! `<ofd:TextCode>` elements. That is the whole format as far as invoice
//! extraction is concerned - which is why this needs no dedicated crate, just
//! the `zip` + `quick-xml` pair.
//!
//! Two paths out, same shape as [`super::pdf`]:
//!
//! 1. **An attachment that is 电子发票 XML.** OFD invoices routinely carry
//!    the original XML under `Doc_0/Attachs/`. Authoritative, so it is tried
//!    first against every XML part in the archive.
//! 2. **The drawn text.** Concatenating the `TextCode` runs gives the same
//!    kind of text blob a PDF text layer does, so it feeds the same
//!    [`crate::parse::text`] field extractor. Unlike a scan, this text is
//!    always real - OFD has no "image-only" mode in practice - which makes
//!    OFD the most reliably parsed format of the three.

use crate::model::Invoice;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::Read;

pub struct OfdContents {
    /// An invoice parsed from an XML part, if one was an 电子发票 document.
    pub xml_invoice: Option<Invoice>,
    /// All drawn text, in page then document order.
    pub text: String,
}

/// Reads an OFD package.
pub fn read(bytes: &[u8]) -> Result<OfdContents, String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("OFD 包无法解压：{e}"))?;

    // Names first: reading a file borrows the archive mutably, so the list
    // has to be taken before the loop.
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();

    let mut xml_invoice = None;
    let mut content_parts: Vec<(String, String)> = Vec::new();

    for name in names {
        if !name.to_ascii_lowercase().ends_with(".xml") {
            continue;
        }
        let Some(text) = read_part(&mut archive, &name) else {
            continue;
        };

        // An attachment that parses as an invoice ends the search - it is
        // strictly better than anything the drawn text can give.
        if xml_invoice.is_none() {
            if let Some(invoice) = super::einvoice_xml::parse(&text) {
                xml_invoice = Some(invoice);
                continue;
            }
        }

        if name.to_ascii_lowercase().contains("content.xml") {
            content_parts.push((name, text));
        }
    }

    // Zip entries come back in archive order, which is not page order.
    // `Pages/Page_10/` must sort after `Page_9/`, so compare the trailing
    // number rather than the string.
    content_parts.sort_by_key(|(name, _)| page_index(name));

    let text = content_parts
        .iter()
        .map(|(_, xml)| text_codes(xml))
        .collect::<Vec<_>>()
        .join("\n");

    Ok(OfdContents { xml_invoice, text })
}

/// The digits in the last `Page_N` style segment, for ordering.
fn page_index(name: &str) -> u32 {
    name.rsplit('/')
        .find_map(|segment| {
            let digits: String = segment.chars().filter(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .unwrap_or(u32::MAX)
}

fn read_part(archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, name: &str) -> Option<String> {
    let file = archive.by_name(name).ok()?;
    // OFD parts are small; the cap is only there so a zip bomb cannot take
    // the process with it.
    let mut buf = Vec::new();
    file.take(16 * 1024 * 1024).read_to_end(&mut buf).ok()?;

    let head = buf.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&buf);
    let declaration = String::from_utf8_lossy(&head[..head.len().min(120)]).to_lowercase();
    let encoding = if declaration.contains("gb2312")
        || declaration.contains("gbk")
        || declaration.contains("gb18030")
    {
        encoding_rs::GB18030
    } else {
        encoding_rs::UTF_8
    };
    Some(encoding.decode(head).0.into_owned())
}

/// Every `<ofd:TextCode>` run in a page, joined with spaces.
///
/// Spaces, not empty string: OFD splits a single visual line into one
/// TextCode per positioned run, and gluing them together would weld a label
/// to its value ("价税合计¥1060.00") and, worse, weld two adjacent numbers
/// into one. A separator keeps the field regexes able to see boundaries.
fn text_codes(xml: &str) -> String {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut out: Vec<String> = Vec::new();
    let mut in_text_code = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name();
                let local = name.as_ref().rsplit(|b| *b == b':').next().unwrap_or(b"");
                in_text_code = local.eq_ignore_ascii_case(b"TextCode");
            }
            Ok(Event::Text(e)) if in_text_code => {
                if let Ok(text) = e.decode() {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        out.push(trimmed.to_string());
                    }
                }
            }
            Ok(Event::End(_)) => in_text_code = false,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn package(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            for (name, body) in parts {
                writer
                    .start_file::<_, ()>(*name, Default::default())
                    .unwrap();
                writer.write_all(body.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }
        buf
    }

    const CONTENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ofd:Page xmlns:ofd="http://www.ofdspec.org/2016">
  <ofd:Content>
    <ofd:TextObject><ofd:TextCode X="10" Y="20">发票号码</ofd:TextCode></ofd:TextObject>
    <ofd:TextObject><ofd:TextCode X="80" Y="20">24312000000012345678</ofd:TextCode></ofd:TextObject>
    <ofd:TextObject><ofd:TextCode X="10" Y="40">价税合计</ofd:TextCode></ofd:TextObject>
    <ofd:TextObject><ofd:TextCode X="80" Y="40">¥1060.00</ofd:TextCode></ofd:TextObject>
  </ofd:Content>
</ofd:Page>"#;

    #[test]
    fn reads_drawn_text_from_page_content() {
        let ofd = package(&[
            ("OFD.xml", "<ofd:OFD/>"),
            ("Doc_0/Pages/Page_0/Content.xml", CONTENT),
        ]);
        let contents = read(&ofd).unwrap();
        assert!(contents.text.contains("24312000000012345678"));
        assert!(contents.text.contains("价税合计"));
    }

    /// Runs must not be welded together, or the field regexes cannot tell
    /// where a label ends and its value begins.
    #[test]
    fn adjacent_runs_stay_separated() {
        let ofd = package(&[("Doc_0/Pages/Page_0/Content.xml", CONTENT)]);
        let text = read(&ofd).unwrap().text;
        assert!(
            text.contains("发票号码 24312000000012345678"),
            "runs were welded: {text}"
        );
    }

    #[test]
    fn an_attached_invoice_xml_wins_over_drawn_text() {
        let attach = r#"<EInvoice><InvoiceNumber>24312000000012345678</InvoiceNumber>
            <TotalTax-includedAmount>1060.00</TotalTax-includedAmount></EInvoice>"#;
        let ofd = package(&[
            ("Doc_0/Attachs/original_invoice.xml", attach),
            ("Doc_0/Pages/Page_0/Content.xml", CONTENT),
        ]);
        let contents = read(&ofd).unwrap();
        let invoice = contents.xml_invoice.expect("attachment parsed");
        assert_eq!(
            invoice.number.value.as_deref(),
            Some("24312000000012345678")
        );
        assert_eq!(invoice.total.confidence, 1.0);
    }

    #[test]
    fn pages_are_ordered_numerically_not_lexically() {
        assert!(
            page_index("Doc_0/Pages/Page_9/Content.xml")
                < page_index("Doc_0/Pages/Page_10/Content.xml")
        );
    }

    #[test]
    fn a_non_zip_is_an_error_not_a_panic() {
        assert!(read(b"%PDF-1.7").is_err());
    }
}
