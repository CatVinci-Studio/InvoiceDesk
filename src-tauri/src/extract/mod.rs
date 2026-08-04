//! The extraction pipeline.
//!
//! One file goes in; an [`Extraction`] comes out. The design principle is
//! that the layers are tried **in descending order of trust, and a better
//! layer's value is never overwritten by a worse one** - which is enforced
//! by [`Field::merge_from`](crate::model::Field::merge_from) rather than by
//! the order of the code here, so a future reordering cannot silently
//! downgrade a field.
//!
//! ```text
//!   PDF  ─┬─ 内嵌 XML 附件 ────────────────▶ 权威，直接结束
//!         ├─ 文本层可用 ──────────────────▶ 正则解析
//!         └─ 扫描件（无文本层）───────────▶ 抽出页面位图 → 图片分支
//!   OFD  ─┬─ 附件 XML ────────────────────▶ 权威，直接结束
//!         └─ 页面绘制文本 ────────────────▶ 正则解析
//!   图片 ──── 二维码 ────────────────────▶ 号码/日期/金额
//!   XML  ──── 直接解析 ──────────────────▶ 权威
//!                     │
//!                     ▼  仍缺关键字段？
//!            vision_image 交给调用方走 VLM
//! ```
//!
//! This module is deliberately **synchronous and IO-free** past the bytes it
//! is handed. The vision fallback needs the network, credentials and an
//! `AppHandle`; rather than drag all of that in here, an incomplete
//! extraction simply hands back the image it could not read, and
//! `commands::ingest` decides whether to spend an API call on it. That keeps
//! the whole of this file unit-testable without a running app.

pub mod detect;
pub mod einvoice_xml;
pub mod ofd;
pub mod pdf;
pub mod qrcode;

use crate::model::{FieldSource, Invoice};
use detect::FileKind;
use image::DynamicImage;
use sha2::{Digest, Sha256};

/// One step of what the pipeline tried, for the "识别过程" disclosure in the
/// detail pane. Users ask "where did this number come from?" and an app that
/// handles money owes them an answer.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceStep {
    /// "PDF 内嵌 XML", "二维码", ...
    pub layer: &'static str,
    /// Whether this layer contributed anything.
    pub hit: bool,
    pub detail: String,
}

impl TraceStep {
    fn hit(layer: &'static str, detail: impl Into<String>) -> Self {
        TraceStep {
            layer,
            hit: true,
            detail: detail.into(),
        }
    }

    fn miss(layer: &'static str, detail: impl Into<String>) -> Self {
        TraceStep {
            layer,
            hit: false,
            detail: detail.into(),
        }
    }
}

pub struct Extraction {
    pub invoice: Invoice,
    pub trace: Vec<TraceStep>,
    /// The image the offline layers could not finish reading. `Some` only
    /// when [`is_complete`] is false AND there is actually something for a
    /// vision model to look at - so the caller never burns an API call on a
    /// file that has no picture in it.
    pub vision_image: Option<DynamicImage>,
}

/// Whether an invoice has everything a reimbursement sheet needs, at a
/// confidence worth trusting.
///
/// 销售方名称 is included even though the sheet could technically be filled
/// without it: it is what the classifier keys off, and an uncategorised row
/// is a row someone has to touch by hand anyway.
pub fn is_complete(invoice: &Invoice) -> bool {
    !invoice.number.needs_review()
        && !invoice.issued_on.needs_review()
        && !invoice.total.needs_review()
        && invoice.seller_name.is_present()
}

pub fn file_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Runs the pipeline over one file's bytes.
pub fn extract(bytes: &[u8], path: &std::path::Path) -> Extraction {
    let mut trace = Vec::new();
    let kind = detect::detect(bytes, path);
    trace.push(TraceStep::hit("文件类型", kind.label()));

    let (mut invoice, vision_image) = match kind {
        FileKind::Pdf => from_pdf(bytes, &mut trace),
        FileKind::Ofd => from_ofd(bytes, &mut trace),
        FileKind::Image => from_image(bytes, &mut trace),
        FileKind::Xml => from_xml(bytes, &mut trace),
        FileKind::Zip => {
            trace.push(TraceStep::miss("压缩包", "需要先解压，逐个导入"));
            (Invoice::default(), None)
        }
        FileKind::Unknown => {
            trace.push(TraceStep::miss("未知格式", "无法识别的文件类型"));
            (Invoice::default(), None)
        }
    };

    invoice.source_path = path.to_string_lossy().into_owned();
    invoice.file_hash = file_hash(bytes);
    crate::parse::validate::infer_missing(&mut invoice);
    invoice.issues = crate::parse::validate::check(&invoice, &Default::default());

    // Only offer the image to the vision layer if it is actually needed.
    let vision_image = vision_image.filter(|_| !is_complete(&invoice));

    Extraction {
        invoice,
        trace,
        vision_image,
    }
}

fn from_pdf(bytes: &[u8], trace: &mut Vec<TraceStep>) -> (Invoice, Option<DynamicImage>) {
    let contents = match pdf::read(bytes) {
        Ok(c) => c,
        Err(e) => {
            trace.push(TraceStep::miss("PDF", e));
            return (Invoice::default(), None);
        }
    };

    if let Some(invoice) = contents.xml_invoice {
        trace.push(TraceStep::hit(
            "PDF 内嵌 XML",
            "找到发票 XML 附件，字段权威",
        ));
        return (invoice, None);
    }
    trace.push(TraceStep::miss("PDF 内嵌 XML", "无 XML 附件"));

    if pdf::has_usable_text(&contents.text) {
        let invoice = crate::parse::text::parse(&contents.text, FieldSource::PdfText);
        trace.push(TraceStep::hit("PDF 文本层", "从文本层解析字段"));

        if is_complete(&invoice) {
            return (invoice, None);
        }
        // The text layer came up short - a QR code in the page image can
        // still supply the identity fields exactly.
        trace.push(TraceStep::miss(
            "PDF 文本层",
            "关键字段不全，继续尝试二维码",
        ));
        return finish_with_images(invoice, pdf::page_images(&contents.document), trace);
    }

    trace.push(TraceStep::miss("PDF 文本层", "无文本层，按扫描件处理"));
    finish_with_images(
        Invoice::default(),
        pdf::page_images(&contents.document),
        trace,
    )
}

fn from_ofd(bytes: &[u8], trace: &mut Vec<TraceStep>) -> (Invoice, Option<DynamicImage>) {
    let contents = match ofd::read(bytes) {
        Ok(c) => c,
        Err(e) => {
            trace.push(TraceStep::miss("OFD", e));
            return (Invoice::default(), None);
        }
    };

    if let Some(invoice) = contents.xml_invoice {
        trace.push(TraceStep::hit("OFD 附件 XML", "找到发票 XML，字段权威"));
        return (invoice, None);
    }

    let invoice = crate::parse::text::parse(&contents.text, FieldSource::PdfText);
    trace.push(if invoice.number.is_present() {
        TraceStep::hit("OFD 版面文本", "从页面文本解析字段")
    } else {
        TraceStep::miss("OFD 版面文本", "页面文本中未找到发票号码")
    });
    (invoice, None)
}

fn from_image(bytes: &[u8], trace: &mut Vec<TraceStep>) -> (Invoice, Option<DynamicImage>) {
    let Ok(image) = image::load_from_memory(bytes) else {
        trace.push(TraceStep::miss("图片", "无法解码这张图片"));
        return (Invoice::default(), None);
    };
    finish_with_images(Invoice::default(), vec![image], trace)
}

fn from_xml(bytes: &[u8], trace: &mut Vec<TraceStep>) -> (Invoice, Option<DynamicImage>) {
    // Tax platforms ship both UTF-8 and GB18030; guessing wrong loses every
    // Chinese name in the document.
    let head = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let declaration = String::from_utf8_lossy(&head[..head.len().min(120)]).to_lowercase();
    let encoding = if declaration.contains("gb2312")
        || declaration.contains("gbk")
        || declaration.contains("gb18030")
    {
        encoding_rs::GB18030
    } else {
        encoding_rs::UTF_8
    };
    let text = encoding.decode(head).0.into_owned();

    match einvoice_xml::parse(&text) {
        Some(invoice) => {
            trace.push(TraceStep::hit("发票 XML", "字段权威"));
            (invoice, None)
        }
        None => {
            trace.push(TraceStep::miss("发票 XML", "不是可识别的电子发票 XML"));
            (Invoice::default(), None)
        }
    }
}

/// Runs the QR decoder over candidate images and folds in whatever it finds,
/// returning the best image for a possible vision pass.
fn finish_with_images(
    mut invoice: Invoice,
    images: Vec<DynamicImage>,
    trace: &mut Vec<TraceStep>,
) -> (Invoice, Option<DynamicImage>) {
    if images.is_empty() {
        trace.push(TraceStep::miss("图像", "文件中没有可用的位图"));
        return (invoice, None);
    }

    let mut decoded = false;
    for image in &images {
        if let Some(qr) = qrcode::decode(image) {
            qr.apply(&mut invoice);
            decoded = true;
            break;
        }
    }

    trace.push(if decoded {
        TraceStep::hit("二维码", "解出发票号码与金额")
    } else {
        TraceStep::miss("二维码", "未找到可识别的发票二维码")
    });

    // The first page is the invoice; later pages are attachments or the
    // 销货清单. Handing the vision model page one keeps the prompt small and
    // the answer focused.
    let candidate = images.into_iter().next();
    (invoice, candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Field, InvoiceKind, Money};
    use std::path::Path;

    #[test]
    fn an_xml_file_is_recognised_and_parsed() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
            <EInvoice>
              <InvoiceNumber>24312000000012345678</InvoiceNumber>
              <IssueTime>2024-03-01</IssueTime>
              <SellerName>杭州云栖酒店管理有限公司</SellerName>
              <TotalTax-includedAmount>1060.00</TotalTax-includedAmount>
            </EInvoice>"#;
        let out = extract(xml.as_bytes(), Path::new("invoice.xml"));
        assert_eq!(
            out.invoice.number.value.as_deref(),
            Some("24312000000012345678")
        );
        assert_eq!(out.invoice.total.value, Some(Money(106_000)));
        assert!(is_complete(&out.invoice));
        // Complete offline - no reason to spend an API call.
        assert!(out.vision_image.is_none());
    }

    #[test]
    fn every_extraction_records_the_file_identity() {
        let out = extract(b"whatever", Path::new("/tmp/a.bin"));
        assert_eq!(out.invoice.source_path, "/tmp/a.bin");
        assert_eq!(out.invoice.file_hash.len(), 64);
    }

    /// Identical bytes must hash identically regardless of where they came
    /// from - that is what catches the same file imported twice.
    #[test]
    fn the_same_bytes_hash_the_same() {
        assert_eq!(file_hash(b"abc"), file_hash(b"abc"));
        assert_ne!(file_hash(b"abc"), file_hash(b"abd"));
    }

    #[test]
    fn completeness_requires_confidence_not_just_presence() {
        let mut inv = Invoice {
            kind: Some(InvoiceKind::DigitalGeneral),
            number: Field::new("24312000000012345678".to_string(), FieldSource::Xml),
            issued_on: Field::new("2024-03-01".to_string(), FieldSource::Xml),
            total: Field::new(Money(106_000), FieldSource::Xml),
            seller_name: Field::new("某公司".to_string(), FieldSource::Xml),
            ..Default::default()
        };
        assert!(is_complete(&inv));

        // Same value, read by a vision model instead - not good enough to
        // go into a reimbursement sheet unreviewed.
        inv.total = Field::new(Money(106_000), FieldSource::Vision);
        assert!(!is_complete(&inv));
    }

    #[test]
    fn a_zip_is_reported_as_needing_expansion_rather_than_failing() {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            w.start_file::<_, ()>("a.pdf", Default::default()).unwrap();
            w.finish().unwrap();
        }
        let out = extract(&buf, Path::new("batch.zip"));
        assert!(out.trace.iter().any(|s| s.layer == "压缩包"));
    }

    #[test]
    fn garbage_produces_issues_rather_than_an_error() {
        let out = extract(b"this is not an invoice", Path::new("note.txt"));
        assert!(!out.invoice.issues.is_empty());
        assert!(!is_complete(&out.invoice));
    }

    /// The trace is what the detail pane shows when a user asks where a
    /// number came from; every run must produce one.
    #[test]
    fn every_run_records_a_trace() {
        let out = extract(b"%PDF-1.4 broken", Path::new("a.pdf"));
        assert!(!out.trace.is_empty());
        assert_eq!(out.trace[0].layer, "文件类型");
    }
}
