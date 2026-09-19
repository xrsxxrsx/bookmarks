//! Format classification for imported files.
//!
//! The UI only distinguishes three classes: `pdf`, `html`, and `other`. EPUB, TXT and
//! everything else deliberately collapse into `other`, but the concrete format is kept
//! alongside the class so the list can show "Other · EPUB" and so a future decision to
//! split EPUB out does not require touching stored data.
//!
//! Detection is content-first (magic bytes, then markup sniffing) with the extension as
//! a tie-breaker, because a mislabelled extension is common in downloaded files while
//! the leading bytes are not ambiguous.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The coarse three-way class the interface works with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FormatClass {
    Pdf,
    Html,
    Other,
}

impl FormatClass {
    pub fn as_str(self) -> &'static str {
        match self {
            FormatClass::Pdf => "pdf",
            FormatClass::Html => "html",
            FormatClass::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileFormat {
    Pdf,
    Html,
    Epub,
    Txt,
    /// Recognised as none of the above. Carries the extension, lower-cased, when there
    /// is one (`azw3`, `mobi`, `docx`, ...), so nothing is silently lost.
    Other(Option<String>),
}

impl FileFormat {
    pub fn class(&self) -> FormatClass {
        match self {
            FileFormat::Pdf => FormatClass::Pdf,
            FileFormat::Html => FormatClass::Html,
            FileFormat::Epub | FileFormat::Txt | FileFormat::Other(_) => FormatClass::Other,
        }
    }

    /// The concrete format for storage and display.
    pub fn detail(&self) -> String {
        match self {
            FileFormat::Pdf => "pdf".into(),
            FileFormat::Html => "html".into(),
            FileFormat::Epub => "epub".into(),
            FileFormat::Txt => "txt".into(),
            FileFormat::Other(Some(ext)) => ext.clone(),
            // No extension at all. Kept as `bin` rather than a word like "unknown" so it is
            // already a safe filename component — the store sanitises whatever it is given,
            // and "unknown" would be stripped to nothing and replaced with this anyway.
            FileFormat::Other(None) => "bin".into(),
        }
    }

    /// Whether the bytes should be decoded and used as searchable text.
    ///
    /// PDF is excluded on purpose: reliable text extraction needs a dedicated engine
    /// and fails outright on scanned files, so a PDF contributes a file record and a
    /// manually entered word count rather than a guessed one.
    pub fn is_text(&self) -> bool {
        matches!(self, FileFormat::Html | FileFormat::Epub | FileFormat::Txt)
    }
}

/// Lower-cased extension, or `None` when the path has none.
pub fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| !e.is_empty())
}

/// Detects the format from content, falling back to the extension.
pub fn detect(path: &Path, bytes: &[u8]) -> FileFormat {
    if bytes.starts_with(b"%PDF-") {
        return FileFormat::Pdf;
    }
    // EPUB is a ZIP container; the mimetype entry is stored first and uncompressed, so
    // it is visible in the first bytes.
    if looks_like_epub(bytes) {
        return FileFormat::Epub;
    }

    let ext = extension_of(path);
    let sniffed_text = sniff_markup(bytes);
    match sniffed_text {
        Some(true) => return FileFormat::Html,
        Some(false) => {
            // Content says plain text; trust the extension only to refine which text
            // format it is, not to override the content.
            return match ext.as_deref() {
                Some("txt" | "text") => FileFormat::Txt,
                _ => FileFormat::Txt,
            };
        }
        None => {}
    }

    match ext.as_deref() {
        Some("pdf") => FileFormat::Pdf,
        Some("epub") => FileFormat::Epub,
        Some("html" | "htm" | "xhtml") => FileFormat::Html,
        Some("txt" | "text") => FileFormat::Txt,
        other => FileFormat::Other(other.map(str::to_owned)),
    }
}

fn looks_like_epub(bytes: &[u8]) -> bool {
    bytes.len() > 58
        && bytes.starts_with(b"PK\x03\x04")
        && bytes[..bytes.len().min(200)]
            .windows(8)
            .any(|w| w == b"mimetype")
}

/// Returns `Some(true)` for HTML-looking text, `Some(false)` for clear plain text, and
/// `None` when the bytes are not decodable as text (so the extension decides).
fn sniff_markup(bytes: &[u8]) -> Option<bool> {
    let head = &bytes[..bytes.len().min(2048)];
    if head.contains(&0) {
        return None; // NUL bytes mean binary, not text
    }
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    let trimmed = text.trim_start();

    if trimmed.starts_with("<!doctype html")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<?xml")
        || trimmed.starts_with("<head")
        || trimmed.contains("<body")
    {
        return Some(true);
    }
    if trimmed.contains("<p>") || trimmed.contains("<div") || trimmed.contains("<br") {
        return Some(true);
    }
    if trimmed.is_empty() {
        return None;
    }
    let printable = head
        .iter()
        .filter(|b| {
            **b == b'\n' || **b == b'\r' || **b == b'\t' || (0x20..0x7f).contains(*b) || **b >= 0x80
        })
        .count();
    if printable * 100 / head.len().max(1) > 95 {
        Some(false)
    } else {
        None
    }
}

/// Decodes bytes to text, handling a BOM and the common non-UTF-8 encodings.
///
/// AO3 serves UTF-8, but files recovered from elsewhere are frequently Windows-encoded,
/// and a wrong guess here would corrupt every CJK title in the library.
pub fn decode_text(bytes: &[u8]) -> String {
    // UTF-8 BOM
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    // UTF-16 BOMs: decode explicitly rather than letting the lossy path mangle them.
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (text, _, _) = encoding_rs::UTF_16LE.decode(&bytes[2..]);
        return text.into_owned();
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (text, _, _) = encoding_rs::UTF_16BE.decode(&bytes[2..]);
        return text.into_owned();
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    // encoding_rs sniffs UTF-8 strictly and otherwise falls back to windows-1252,
    // which never fails. Check GBK/Big5/Shift_JIS-style candidates via BOM-less
    // detection only for clearly non-UTF-8 input.
    if let Some((encoding, _)) = encoding_rs::Encoding::for_bom(bytes) {
        let (text, _, _) = encoding.decode(bytes);
        return text.into_owned();
    }
    // No BOM and not UTF-8: try the East Asian encodings that Windows users most often
    // encounter, then fall back to a lossy UTF-8 read so nothing is dropped entirely.
    for encoding in [encoding_rs::GBK, encoding_rs::BIG5, encoding_rs::SHIFT_JIS] {
        let (text, _, had_errors) = encoding.decode(bytes);
        if !had_errors {
            return text.into_owned();
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    #[test]
    fn pdf_detected_by_magic_bytes_despite_wrong_extension() {
        let f = detect(&p("mislabelled.txt"), b"%PDF-1.7\n...");
        assert_eq!(f, FileFormat::Pdf);
        assert_eq!(f.class(), FormatClass::Pdf);
    }

    #[test]
    fn epub_detected_as_zip_container() {
        let mut bytes = b"PK\x03\x04".to_vec();
        bytes.extend_from_slice(&[0u8; 20]);
        bytes.extend_from_slice(b"mimetypeapplication/epub+zip");
        bytes.extend_from_slice(&[0u8; 40]);
        let f = detect(&p("book.epub"), &bytes);
        assert_eq!(f, FileFormat::Epub);
        assert_eq!(
            f.class(),
            FormatClass::Other,
            "epub belongs to the coarse 'other' class"
        );
        assert_eq!(f.detail(), "epub");
    }

    #[test]
    fn html_detected_by_content_not_extension() {
        let f = detect(
            &p("no_extension"),
            b"<!DOCTYPE html>\n<html><body>hi</body></html>",
        );
        assert_eq!(f, FileFormat::Html);
        assert_eq!(f.class(), FormatClass::Html);
    }

    #[test]
    fn plain_text_detected_as_txt() {
        let f = detect(&p("story.txt"), "just some prose\nwith lines".as_bytes());
        assert_eq!(f, FileFormat::Txt);
        assert_eq!(
            f.class(),
            FormatClass::Other,
            "txt is 'other' per the requirement"
        );
        assert_eq!(f.detail(), "txt");
    }

    #[test]
    fn unknown_binary_keeps_its_extension() {
        let f = detect(&p("book.azw3"), &[0u8, 1, 2, 3, 0, 5, 200, 7]);
        assert_eq!(f, FileFormat::Other(Some("azw3".into())));
        assert_eq!(f.class(), FormatClass::Other);
        assert_eq!(f.detail(), "azw3");
    }

    #[test]
    fn format_classes_are_exactly_three() {
        assert_eq!(FileFormat::Pdf.class().as_str(), "pdf");
        assert_eq!(FileFormat::Html.class().as_str(), "html");
        for other in [
            FileFormat::Epub,
            FileFormat::Txt,
            FileFormat::Other(Some("docx".into())),
        ] {
            assert_eq!(other.class().as_str(), "other", "{other:?} must be 'other'");
        }
    }

    #[test]
    fn pdf_is_not_treated_as_searchable_text() {
        assert!(!FileFormat::Pdf.is_text());
        assert!(FileFormat::Html.is_text());
        assert!(FileFormat::Txt.is_text());
    }

    #[test]
    fn decodes_utf8_bom() {
        let bytes = [&[0xEF, 0xBB, 0xBF][..], "粽子".as_bytes()].concat();
        assert_eq!(decode_text(&bytes), "粽子");
    }

    #[test]
    fn decodes_utf8_without_bom() {
        assert_eq!(decode_text("简单地杀个人".as_bytes()), "简单地杀个人");
    }

    #[test]
    fn decodes_gbk_chinese() {
        // "中文" in GBK
        let bytes = [0xD6u8, 0xD0, 0xCE, 0xC4];
        let text = decode_text(&bytes);
        assert_eq!(text, "中文", "GBK input must not be mangled");
    }

    #[test]
    fn decodes_utf16le_with_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "hi".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_text(&bytes), "hi");
    }
}
