//! Domain logic for the local fanfiction library.
//!
//! Layering: this crate has no knowledge of Tauri or of any UI. It owns the data model,
//! the storage schema and all import/parsing logic, so the parsing behaviour can be
//! tested against real files without launching the desktop shell.

pub mod ao3;
pub mod config;
pub mod date;
pub mod db;
pub mod error;
pub mod fileformat;
pub mod hashing;
pub mod import;
pub mod store;
pub mod text;
pub mod view;

pub use error::{Error, Result};

/// Metadata extracted from one imported file, before it is matched to a work.
///
/// The AO3 variant is boxed because it is an order of magnitude larger than the generic
/// one; without the indirection every `Extracted` value — including the common "just a
/// text file" case — would be sized for the richest possible metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extracted {
    /// A file recognised as an AO3 download; the richest source of metadata.
    Ao3(Box<ao3::Ao3Work>),
    /// Any other file. Only the format and a plain-text rendering are known.
    Generic {
        format: fileformat::FileFormat,
        text: Option<String>,
    },
}

impl Extracted {
    /// AO3 work id, when known. Used to group formats of one work together and to
    /// match a work against the relations listed in other files.
    pub fn external_id(&self) -> Option<&str> {
        match self {
            Extracted::Ao3(work) => work.work_id.as_deref(),
            Extracted::Generic { .. } => None,
        }
    }
}

/// Classifies the bytes of a file and parses it when the format is supported.
///
/// Detection is content-based and falls back to the file extension, so a `.html`
/// that is really an EPUB (or vice versa) still lands in the right branch.
pub fn extract(path: &std::path::Path) -> Result<Extracted> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    let format = fileformat::detect(path, &bytes);

    match format {
        fileformat::FileFormat::Html => {
            let html = fileformat::decode_text(&bytes);
            match ao3::parse(&html) {
                // Only treat a file as an AO3 download when the AO3 structure is
                // actually present. Anything else stays plain HTML rather than
                // producing a half-empty record.
                Some(work) if work.is_confident() => Ok(Extracted::Ao3(Box::new(work))),
                _ => Ok(Extracted::Generic {
                    format,
                    text: Some(crate::text::html_to_text(&html)),
                }),
            }
        }
        other => Ok(Extracted::Generic {
            format: other,
            text: None,
        }),
    }
}
