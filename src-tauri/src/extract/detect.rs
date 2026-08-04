//! What kind of file this is, decided by content rather than extension.
//!
//! Extensions lie constantly here: tax platforms hand out `.pdf` files that
//! are really OFD, WeChat renames attachments to `.jpg` regardless of what
//! they hold, and users rename things. Sniffing the magic bytes costs one
//! read of the first few bytes and removes a whole class of "why did nothing
//! happen when I dropped this in" bug reports.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Pdf,
    /// A zip whose central directory contains `OFD.xml` - China's own
    /// GB/T 33190 layout format, and the other half of what the tax
    /// platform issues alongside PDF.
    Ofd,
    /// Any raster image: a photo or scan of a paper invoice.
    Image,
    /// A bare 电子发票 XML file. Tax platforms sometimes deliver the XML on
    /// its own, and it is the most authoritative form there is.
    Xml,
    /// A zip that is not OFD - almost always a batch of invoices the user
    /// downloaded from a mail client, so it is worth unpacking rather than
    /// rejecting.
    Zip,
    Unknown,
}

impl FileKind {
    pub fn label(self) -> &'static str {
        match self {
            FileKind::Pdf => "PDF",
            FileKind::Ofd => "OFD",
            FileKind::Image => "图片",
            FileKind::Xml => "XML",
            FileKind::Zip => "压缩包",
            FileKind::Unknown => "未知格式",
        }
    }
}

const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// Sniffs `bytes`. `path` is consulted only to break ties the content cannot
/// (a zip that is neither OFD nor obviously a batch), never to override it.
pub fn detect(bytes: &[u8], path: &Path) -> FileKind {
    if bytes.starts_with(b"%PDF") {
        return FileKind::Pdf;
    }

    if bytes.starts_with(ZIP_MAGIC) {
        return if zip_contains_ofd_root(bytes) {
            FileKind::Ofd
        } else {
            FileKind::Zip
        };
    }

    // A UTF-8 BOM ahead of the declaration is common in tax-platform XML.
    let head = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if head.starts_with(b"<?xml") || head.starts_with(b"<EInvoice") {
        return FileKind::Xml;
    }

    if image::guess_format(bytes).is_ok() {
        return FileKind::Image;
    }

    // Last resort: an extension we recognise on a file whose header we do
    // not. Better to attempt extraction and fail with a real message than to
    // reject something the user knows is an invoice.
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pdf") => FileKind::Pdf,
        Some("ofd") => FileKind::Ofd,
        Some("xml") => FileKind::Xml,
        Some("jpg" | "jpeg" | "png" | "bmp" | "gif" | "tif" | "tiff" | "webp" | "heic") => {
            FileKind::Image
        }
        _ => FileKind::Unknown,
    }
}

/// Whether a zip archive holds `OFD.xml` at its root - the entry GB/T 33190
/// requires, and the only reliable way to tell an OFD from any other zip.
fn zip_contains_ofd_root(bytes: &[u8]) -> bool {
    let cursor = std::io::Cursor::new(bytes);
    let Ok(archive) = zip::ZipArchive::new(cursor) else {
        return false;
    };
    // Collected rather than iterated: `file_names` borrows the archive, and
    // the archive is a local, so the iterator cannot outlive this expression.
    let names: Vec<&str> = archive.file_names().collect();
    names
        .iter()
        .any(|name| name.eq_ignore_ascii_case("OFD.xml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_wins_on_magic_bytes_whatever_the_extension_says() {
        assert_eq!(
            detect(b"%PDF-1.7\n...", Path::new("invoice.jpg")),
            FileKind::Pdf
        );
    }

    #[test]
    fn xml_declaration_survives_a_bom() {
        let with_bom = b"\xEF\xBB\xBF<?xml version=\"1.0\"?><EInvoice/>";
        assert_eq!(detect(with_bom, Path::new("x.bin")), FileKind::Xml);
    }

    #[test]
    fn unrecognised_content_falls_back_to_the_extension() {
        assert_eq!(detect(b"garbage", Path::new("a.ofd")), FileKind::Ofd);
        assert_eq!(detect(b"garbage", Path::new("a.heic")), FileKind::Image);
        assert_eq!(detect(b"garbage", Path::new("a.txt")), FileKind::Unknown);
    }

    /// A zip with no OFD.xml is a batch download, not an OFD - misfiling it
    /// would send a folder of invoices down the single-document path.
    #[test]
    fn plain_zip_is_not_ofd() {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            w.start_file::<_, ()>("a.pdf", Default::default()).unwrap();
            w.finish().unwrap();
        }
        assert_eq!(detect(&buf, Path::new("batch.zip")), FileKind::Zip);
    }

    #[test]
    fn zip_with_ofd_root_is_ofd() {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            w.start_file::<_, ()>("OFD.xml", Default::default())
                .unwrap();
            w.finish().unwrap();
        }
        assert_eq!(detect(&buf, Path::new("x")), FileKind::Ofd);
    }
}
