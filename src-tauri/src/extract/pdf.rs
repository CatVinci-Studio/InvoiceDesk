//! PDF: the format nearly every Chinese electronic invoice arrives in.
//!
//! Three things can be pulled out of one, in descending order of trust:
//!
//! 1. **An attached 电子发票 XML.** 数电票 are frequently delivered as a PDF
//!    with the authoritative XML tucked into the document's `EmbeddedFiles`
//!    name tree. When it is there, nothing else needs reading - see
//!    [`super::einvoice_xml`].
//! 2. **The text layer.** Electronic invoices are generated PDFs, so the
//!    field values are real text. Free, offline, and exact when the fonts
//!    carry a usable `ToUnicode` map. (Some issuers ship CID fonts without
//!    one, and the extracted "text" is then mojibake - which is why
//!    [`has_usable_text`] checks the result rather than assuming.)
//! 3. **The page images.** A scanned invoice has no text layer at all, but
//!    it does have exactly one big image XObject per page. Pulling that out
//!    hands the QR decoder and the vision model something to work with
//!    WITHOUT bundling a PDF rasteriser - see [`page_images`].
//!
//! Point 3 is worth dwelling on. The obvious way to handle scanned PDFs is
//! to render them, which means shipping pdfium: ~4 MB per platform, a build
//! dependency on every CI runner, and a second binary to keep signed. But a
//! scanned invoice is a photo that someone wrapped in a PDF - the original
//! JPEG is sitting right there in the object graph, at full resolution.
//! Extracting it is a few dozen lines and gives a BETTER image than
//! rendering would, because there is no re-compression step.

use crate::model::Invoice;
use image::DynamicImage;
use lopdf::{Dictionary, Document, Object};

/// Everything one PDF yielded.
pub struct PdfContents {
    /// An invoice parsed from an embedded XML attachment, if there was one.
    pub xml_invoice: Option<Invoice>,
    /// The text layer, empty when there is none (or none that decoded).
    pub text: String,
    /// Page images, for the scanned case. Lazily produced - see
    /// [`page_images`] - so a text-layer PDF never pays for them.
    pub document: Document,
}

/// Reads a PDF into its extractable parts.
pub fn read(bytes: &[u8]) -> Result<PdfContents, String> {
    let document = Document::load_mem(bytes).map_err(|e| format!("PDF 无法解析：{e}"))?;

    let xml_invoice = embedded_xml(&document)
        .iter()
        .find_map(|xml| super::einvoice_xml::parse(xml));

    Ok(PdfContents {
        xml_invoice,
        text: extract_text(bytes),
        document,
    })
}

/// Whether the extracted text is actually usable, as opposed to present.
///
/// A scanned PDF yields "" and a broken-font PDF yields a string of
/// replacement characters and private-use codepoints; both must fall through
/// to the image path rather than being regexed for fields that cannot be
/// there. The test is simply whether enough CJK or ASCII made it out -
/// invoice text layers are dense with both.
pub fn has_usable_text(text: &str) -> bool {
    let legible = text
        .chars()
        .filter(|c| {
            c.is_ascii_alphanumeric()
                // CJK Unified Ideographs - what an invoice is mostly made of.
                || ('\u{4E00}'..='\u{9FFF}').contains(c)
        })
        .count();
    legible >= 40
}

/// Runs pdf-extract, converting a panic into an error.
///
/// `catch_unwind` is not idiomatic and is used deliberately: this is a BATCH
/// importer, and pdf-extract indexes into font tables that malformed
/// real-world PDFs do not always populate. One bad file out of two hundred
/// must degrade to "this one needs review", never take down the import.
fn extract_text(bytes: &[u8]) -> String {
    let owned = bytes.to_vec();
    std::panic::catch_unwind(move || pdf_extract::extract_text_from_mem(&owned).ok())
        .ok()
        .flatten()
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Embedded file attachments
// ---------------------------------------------------------------------------

/// Follows `Object::Reference` links until something concrete is reached.
fn resolve<'a>(doc: &'a Document, object: &'a Object) -> &'a Object {
    let mut current = object;
    // Bounded rather than `loop`: a corrupt file can contain a reference
    // cycle, and this runs over untrusted input.
    for _ in 0..32 {
        match current {
            Object::Reference(id) => match doc.get_object(*id) {
                Ok(next) => current = next,
                Err(_) => return current,
            },
            _ => return current,
        }
    }
    current
}

/// Every embedded file whose bytes look like XML, decoded to a string.
///
/// Walks the catalog's `Names → EmbeddedFiles` name tree (including `Kids`
/// subtrees, which large PDFs use) and the per-page `FileAttachment`
/// annotations, because issuers use both.
fn embedded_xml(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();

    if let Ok(catalog) = doc.catalog() {
        if let Some(names) = catalog
            .get(b"Names")
            .ok()
            .map(|o| resolve(doc, o))
            .and_then(|o| o.as_dict().ok())
        {
            if let Some(tree) = names
                .get(b"EmbeddedFiles")
                .ok()
                .map(|o| resolve(doc, o))
                .and_then(|o| o.as_dict().ok())
            {
                collect_name_tree(doc, tree, &mut out, 0);
            }
        }
    }

    for (_, page_id) in doc.get_pages() {
        let Ok(page) = doc.get_object(page_id).and_then(|o| o.as_dict().cloned()) else {
            continue;
        };
        let Some(annots) = page
            .get(b"Annots")
            .ok()
            .map(|o| resolve(doc, o))
            .and_then(|o| o.as_array().ok())
        else {
            continue;
        };
        for annot in annots {
            if let Ok(dict) = resolve(doc, annot).as_dict() {
                if let Ok(fs) = dict.get(b"FS") {
                    if let Ok(spec) = resolve(doc, fs).as_dict() {
                        push_filespec(doc, spec, &mut out);
                    }
                }
            }
        }
    }

    out
}

/// A PDF name tree is `{Names: [key, value, ...]}` or `{Kids: [subtree...]}`.
/// Depth is bounded because the structure is attacker-controlled.
fn collect_name_tree(doc: &Document, node: &Dictionary, out: &mut Vec<String>, depth: usize) {
    if depth > 8 {
        return;
    }

    if let Some(names) = node
        .get(b"Names")
        .ok()
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_array().ok())
    {
        // Alternating [name, filespec] pairs; only the filespecs matter.
        for entry in names.iter().skip(1).step_by(2) {
            if let Ok(spec) = resolve(doc, entry).as_dict() {
                push_filespec(doc, spec, out);
            }
        }
    }

    if let Some(kids) = node
        .get(b"Kids")
        .ok()
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_array().ok())
    {
        for kid in kids {
            if let Ok(dict) = resolve(doc, kid).as_dict() {
                collect_name_tree(doc, dict, out, depth + 1);
            }
        }
    }
}

/// Pulls the stream out of a `/Filespec` (`EF → F` or `UF`) and keeps it if
/// it decodes to XML.
fn push_filespec(doc: &Document, spec: &Dictionary, out: &mut Vec<String>) {
    let Some(ef) = spec
        .get(b"EF")
        .ok()
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_dict().ok())
    else {
        return;
    };

    for key in [b"F".as_slice(), b"UF".as_slice()] {
        let Ok(entry) = ef.get(key) else { continue };
        let Ok(stream) = resolve(doc, entry).as_stream() else {
            continue;
        };
        let Ok(bytes) = stream.decompressed_content() else {
            continue;
        };
        if let Some(text) = decode_xml(&bytes) {
            out.push(text);
            return;
        }
    }
}

/// Decodes attachment bytes as XML, honouring the encoding actually used.
///
/// Chinese tax platforms ship both UTF-8 and GB18030 XML, and a GB18030
/// document read as UTF-8 loses every Chinese name in it - which is most of
/// the fields worth having.
fn decode_xml(bytes: &[u8]) -> Option<String> {
    let head = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if !head.starts_with(b"<?xml") && !head.starts_with(b"<") {
        return None;
    }

    // The declaration names the encoding, and it is pure ASCII either way.
    let declaration = String::from_utf8_lossy(&head[..head.len().min(120)]).to_lowercase();
    let encoding = if declaration.contains("gb2312")
        || declaration.contains("gbk")
        || declaration.contains("gb18030")
    {
        encoding_rs::GB18030
    } else {
        encoding_rs::UTF_8
    };

    let (text, _, had_errors) = encoding.decode(head);
    (!had_errors || text.contains('<')).then(|| text.into_owned())
}

// ---------------------------------------------------------------------------
// Page images (the scanned case)
// ---------------------------------------------------------------------------

/// The largest image on each page, decoded.
///
/// Scanned invoices are one full-page image; returning the largest per page
/// skips issuer logos and the red 发票专用章 stamp graphic without needing to
/// understand the page layout.
///
/// Only `DCTDecode` (JPEG) and unfiltered/`FlateDecode` bitmaps are handled.
/// `JPXDecode` and `CCITTFaxDecode` would each pull in a codec for a case
/// that barely occurs in invoices; they are skipped, and the caller reports
/// the file as needing manual entry rather than pretending it scanned.
pub fn page_images(doc: &Document) -> Vec<DynamicImage> {
    let mut out = Vec::new();

    for (_, page_id) in doc.get_pages() {
        let mut best: Option<(u64, DynamicImage)> = None;

        for (_, stream) in xobjects(doc, page_id) {
            let Some(image) = decode_image_stream(&stream) else {
                continue;
            };
            let pixels = u64::from(image.width()) * u64::from(image.height());
            // A logo is a few thousand pixels; a scan is millions. The floor
            // keeps stamps and decorations out.
            if pixels < 200_000 {
                continue;
            }
            if best.as_ref().is_none_or(|(area, _)| pixels > *area) {
                best = Some((pixels, image));
            }
        }

        if let Some((_, image)) = best {
            out.push(image);
        }
    }

    out
}

/// Every image XObject reachable from a page's resources.
fn xobjects(doc: &Document, page_id: lopdf::ObjectId) -> Vec<(String, lopdf::Stream)> {
    let mut out = Vec::new();
    let Ok(page) = doc.get_object(page_id).and_then(|o| o.as_dict().cloned()) else {
        return out;
    };
    let Some(resources) = page
        .get(b"Resources")
        .ok()
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_dict().ok())
    else {
        return out;
    };
    let Some(xobject) = resources
        .get(b"XObject")
        .ok()
        .map(|o| resolve(doc, o))
        .and_then(|o| o.as_dict().ok())
    else {
        return out;
    };

    for (name, value) in xobject.iter() {
        if let Ok(stream) = resolve(doc, value).as_stream() {
            let is_image = stream
                .dict
                .get(b"Subtype")
                .ok()
                .and_then(|s| s.as_name().ok())
                .is_some_and(|n| n == b"Image");
            if is_image {
                out.push((String::from_utf8_lossy(name).into_owned(), stream.clone()));
            }
        }
    }
    out
}

fn decode_image_stream(stream: &lopdf::Stream) -> Option<DynamicImage> {
    let filters = stream.filters().unwrap_or_default();
    let is_jpeg = filters.iter().any(|f| *f == b"DCTDecode".as_slice());

    if is_jpeg {
        // JPEG data is stored as-is inside the stream, so the raw bytes are
        // a complete JPEG file - no reconstruction needed.
        return image::load_from_memory_with_format(&stream.content, image::ImageFormat::Jpeg).ok();
    }

    // Otherwise the stream is a raw sample grid and the dictionary describes
    // its shape.
    let bytes = stream.decompressed_content().ok()?;
    let dict = &stream.dict;
    let width: u32 = dict.get(b"Width").ok()?.as_i64().ok()?.try_into().ok()?;
    let height: u32 = dict.get(b"Height").ok()?.as_i64().ok()?.try_into().ok()?;
    let bpc = dict
        .get(b"BitsPerComponent")
        .ok()
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(8);
    if bpc != 8 {
        return None;
    }

    let pixels = (width as usize).checked_mul(height as usize)?;
    match bytes.len().checked_div(pixels) {
        Some(1) => image::GrayImage::from_raw(width, height, bytes).map(DynamicImage::ImageLuma8),
        Some(3) => image::RgbImage::from_raw(width, height, bytes).map(DynamicImage::ImageRgb8),
        Some(4) => image::RgbaImage::from_raw(width, height, bytes).map(DynamicImage::ImageRgba8),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usable_text_rejects_an_empty_or_garbled_layer() {
        assert!(!has_usable_text(""));
        assert!(!has_usable_text("   \n \n  "));
        // What a CID font with no ToUnicode map produces.
        assert!(!has_usable_text(&"\u{FFFD}".repeat(200)));
        assert!(!has_usable_text(&"\u{E000}".repeat(200)));
    }

    #[test]
    fn usable_text_accepts_a_real_invoice_layer() {
        let layer = "电子发票（普通发票）发票号码 24312000000012345678 开票日期 2024年03月01日 \
                     购买方名称 猫芬奇工作室 销售方名称 杭州云栖酒店管理有限公司 价税合计 ¥1060.00";
        assert!(has_usable_text(layer));
    }

    #[test]
    fn xml_attachments_decode_from_gb18030() {
        // "<?xml version="1.0" encoding="GB18030"?><X>中</X>" in GB18030 -
        // read as UTF-8 the Chinese character would be lost.
        let mut bytes = b"<?xml version=\"1.0\" encoding=\"GB18030\"?><X>".to_vec();
        bytes.extend_from_slice(&[0xD6, 0xD0]); // 中
        bytes.extend_from_slice(b"</X>");
        let decoded = decode_xml(&bytes).expect("decodes");
        assert!(decoded.contains('中'), "GB18030 中文未正确解码: {decoded}");
    }

    #[test]
    fn non_xml_attachments_are_ignored() {
        assert_eq!(decode_xml(b"%PDF-1.4"), None);
        assert_eq!(decode_xml(&[0xFF, 0xD8, 0xFF]), None);
    }

    #[test]
    fn broken_pdfs_error_rather_than_panic() {
        assert!(read(b"not a pdf at all").is_err());
    }
}
