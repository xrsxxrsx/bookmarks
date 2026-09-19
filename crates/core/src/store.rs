//! The managed library: the SQLite database plus the directory of imported files.
//!
//! The two live under one root so that "back up my library" is a single directory copy
//! that cannot half-succeed. Files inside the root are named by work id, never by title:
//!
//! ```text
//! <root>/
//!   bookmarks.db
//!   library/
//!     000001/
//!       work.html
//!       work.epub
//!     000002/
//!       work.txt
//! ```
//!
//! Two decisions worth stating, because both remove whole classes of bug:
//!
//!  * **Directories are numbered by work id, not titled.** A title can contain characters
//!    Windows forbids, can collide case-insensitively, can exceed the path limit, and
//!    changes when the user edits it — each of which would mean renaming directories and
//!    rewriting stored paths. The title lives only in the database.
//!  * **Files are named `work.<ext>`.** Several formats of one work therefore coexist
//!    without `(1)`/`(2)` suffixes, and re-importing overwrites in place. The name the
//!    file arrived with is kept in the database for display.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{Error, Result};
use crate::fileformat::{FileFormat, FormatClass};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecord {
    pub id: i64,
    pub work_id: i64,
    pub format_class: FormatClass,
    pub format_detail: String,
    pub original_name: String,
    pub rel_path: String,
    pub file_hash: String,
    pub size_bytes: i64,
    pub word_count: Option<i64>,
    pub is_estimated: bool,
    /// `current` or `archived`. Exactly one `current` per (work, format).
    pub role: String,
    /// 1 for the original, 2 for the first replacement kept alongside it, and so on.
    pub version: i64,
}

impl FileRecord {
    pub fn is_current(&self) -> bool {
        self.role == "current"
    }
}

/// A work's membership in one series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesMembership {
    pub name: String,
    /// Position in the series, when the order is known. The interface sorts unknown positions
    /// last rather than inventing a number.
    pub position: Option<i64>,
}

/// A translation suggestion the user has dismissed, kept so it can be restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DismissedPending {
    pub external_id: String,
    pub title: Option<String>,
    pub author: Option<String>,
}

/// What to do when a file of the same format is already stored for a work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileVersionPolicy {
    /// The stored file becomes `archived` and the new one becomes `current`.
    ///
    /// The default, because the usual reason to re-import is that the work was updated or
    /// reposted, and losing the earlier revision is worse than carrying a second copy.
    KeepBoth,
    /// The stored file is replaced, one slot per format. This was the only behaviour before
    /// versions existed.
    ReplaceOnly,
}

/// The result of storing a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreOutcome {
    Stored(Box<FileRecord>),
    /// The bytes were already present, so nothing was copied. This is the safety net for the
    /// case the import preview could not catch: the same content already stored in a
    /// *different* format slot, which would otherwise end up in the library twice.
    DuplicateOf {
        work_id: i64,
        file_id: i64,
    },
}

impl StoreOutcome {
    pub fn into_file(self) -> Option<FileRecord> {
        match self {
            StoreOutcome::Stored(record) => Some(*record),
            StoreOutcome::DuplicateOf { .. } => None,
        }
    }
}

/// A new work to insert.
#[derive(Debug, Clone, Default)]
pub struct NewWork {
    pub title: String,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub my_comment: Option<String>,
    /// Normalised ISO date; partial dates are filled to the start of their range.
    pub published_at: Option<String>,
    pub published_prec: Option<String>,
    pub date_is_approx: bool,
    pub date_source: Option<String>,
    pub completed_at: Option<String>,
    pub word_count: i64,
    pub word_count_source: Option<String>,
    pub language: Option<String>,
    pub language_label: Option<String>,
    pub translation_role: Option<String>,
    pub source_url: Option<String>,
    pub external_id: Option<String>,
    /// JSON array of AO3's own tags, kept apart from the user's tags.
    pub ao3_tags: Option<String>,
}

/// A date being set from the interface.
///
/// Accepts the loose strings a person actually types (`2019`, `2019-06`, `2019-06-09`) and
/// derives the precision from what was given, so the interface does not have to send a
/// precision the user never chose. `approx` marks a value the user guessed rather than read
/// off a source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DateEdit {
    pub input: String,
    pub approx: bool,
}

impl DateEdit {
    /// `(iso, precision)` when the input is usable, or `None` when it should clear the
    /// field. Anything unparseable is rejected by the caller rather than stored as a guess,
    /// because a silently mangled date is worse than an error message.
    pub fn parse(&self) -> Result<Option<(String, String, bool)>> {
        let trimmed = self.input.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        match crate::date::DateValue::parse_loose(trimmed, self.approx) {
            Some(value) => Ok(Some((
                value.iso,
                value.precision.as_str().to_owned(),
                value.is_approx,
            ))),
            None => Err(Error::Invalid(format!(
                "“{trimmed}” 不是可识别的日期；请用 2019、2019-06 或 2019-06-09 这样的格式"
            ))),
        }
    }
}

/// Every editable field of a work, as one atomic change.
///
/// A field passed as `None` is left alone, so a form that does not show a field cannot
/// blank it by omission. Strings and dates are cleared by sending an empty value, which is
/// an explicit action rather than an accident.
#[derive(Debug, Clone, Default)]
pub struct WorkEdit {
    pub title: Option<String>,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub my_comment: Option<String>,
    pub published: Option<DateEdit>,
    pub completed: Option<DateEdit>,
    /// `has_file` | `record_only`
    pub status: Option<String>,
    /// ISO 639-1 code typed by the user, for files that carry no language metadata — a bare TXT,
    /// or a format the importer cannot read text from.
    pub language: Option<String>,
    /// The display form of the language, which the interface shows next to the code.
    pub language_label: Option<String>,
    /// A word count typed by the user. Stored with `word_count_source = 'manual'`, so it is
    /// displayed exactly like AO3's own figure rather than marked as an estimate — the whole
    /// point is that the user knows the number.
    pub word_count: Option<i64>,
    /// The complete intended tag set, replacing whatever is stored.
    pub tags: Option<Vec<String>>,
}

pub struct Library {
    conn: Connection,
    root: PathBuf,
}

impl Library {
    /// Opens a library, creating the root and migrating the schema if needed.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(root.join("library")).map_err(|e| Error::io(&root, e))?;
        let conn = crate::db::open(crate::db::default_path(&root))?;
        Ok(Library { conn, root })
    }

    /// An in-memory database with a temporary file store, for tests.
    #[cfg(test)]
    pub fn open_in_memory(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(root.join("library")).map_err(|e| Error::io(&root, e))?;
        let conn = crate::db::open_in_memory()?;
        Ok(Library { conn, root })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory holding one work's files. Not created until a file is stored.
    pub fn work_dir(&self, work_id: i64) -> PathBuf {
        self.root.join("library").join(format!("{work_id:06}"))
    }

    /// Relative path a file of this format occupies within a work directory.
    ///
    /// One path per format, so a work's html and epub sit side by side and re-importing
    /// a format replaces it rather than accumulating duplicates.
    pub fn rel_path_for(work_id: i64, format: &FileFormat) -> String {
        format!(
            "{:06}/work.{}",
            work_id,
            sanitize_extension(&format.detail())
        )
    }

    pub fn abs_path(&self, rel_path: &str) -> PathBuf {
        self.root.join("library").join(rel_path)
    }

    /// Path for version `n` of a format, used when earlier versions are kept.
    ///
    /// Version 1 keeps the natural `work.<ext>` name and later versions gain a `.vN` suffix,
    /// so a work that never versions looks exactly as before and the numbering is visible in
    /// the directory listing.
    pub fn versioned_rel_path(work_id: i64, format: &FileFormat, version: i64) -> String {
        if version <= 1 {
            return Self::rel_path_for(work_id, format);
        }
        let ext = sanitize_extension(&format.detail());
        format!("{:06}/work.v{}.{}", work_id, version, ext)
    }

    // ---------------------------------------------------------------- works

    /// Inserts a work and returns its id.
    pub fn insert_work(&self, work: &NewWork) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO works (
                 title, author, summary, my_comment,
                 published_at, published_prec, date_is_approx, date_source, completed_at,
                 word_count, word_count_source,
                 language, language_label, translation_role,
                 source_url, external_id, ao3_tags, status,
                 created_at, updated_at
             ) VALUES (
                 ?1, ?2, ?3, ?4,
                 ?5, ?6, ?7, ?8, ?9,
                 ?10, ?11,
                 ?12, ?13, ?14,
                 ?15, ?16, ?17, 'has_file',
                 datetime('now'), datetime('now')
             )",
            params![
                work.title,
                work.author,
                work.summary,
                work.my_comment,
                work.published_at,
                work.published_prec,
                work.date_is_approx as i64,
                work.date_source,
                work.completed_at,
                work.word_count,
                work.word_count_source,
                work.language,
                work.language_label,
                work.translation_role,
                work.source_url,
                work.external_id,
                work.ao3_tags,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Looks up an existing work by AO3 work id.
    pub fn find_work_by_external_id(&self, external_id: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM works WHERE external_id = ?1",
                params![external_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Work that already owns a file with this content, used to spot a re-import.
    pub fn find_work_by_file_hash(&self, hash: &str) -> Result<Option<(i64, i64)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT work_id, id FROM files WHERE file_hash = ?1 LIMIT 1",
                params![hash],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// Finds a work whose title matches under the same folding used for search, so a file
    /// with no AO3 id can still join the work already in the library.
    ///
    /// Titles are compared with [`crate::text::normalize`] in Rust rather than in SQL: the
    /// folding rules (NFKC, punctuation removal, case) have no SQL equivalent, and a
    /// personal library is small enough that this is not worth indexing.
    pub fn find_work_by_title(&self, title: &str) -> Result<Option<i64>> {
        let wanted = crate::text::normalize(title);
        if wanted.is_empty() {
            return Ok(None);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT id, title FROM works WHERE title IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, stored_title) = row?;
            if crate::text::normalize(&stored_title) == wanted {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// Fills in fields that are still empty, so importing another file of the same work
    /// can complete metadata the first file did not carry (a TXT has no summary, an HTML
    /// later supplies one). Existing values are never overwritten: a user's manual edit
    /// must survive a re-import.
    pub fn fill_missing_metadata(&self, work_id: i64, work: &NewWork) -> Result<()> {
        self.conn.execute(
            "UPDATE works SET
                 author            = COALESCE(author, ?2),
                 summary           = COALESCE(summary, ?3),
                 published_at      = COALESCE(published_at, ?4),
                 published_prec    = COALESCE(published_prec, ?5),
                 date_source       = COALESCE(date_source, ?6),
                 completed_at      = COALESCE(completed_at, ?7),
                 word_count        = CASE WHEN word_count > 0 THEN word_count ELSE ?8 END,
                 word_count_source = COALESCE(word_count_source, ?9),
                 language          = COALESCE(language, ?10),
                 language_label    = COALESCE(language_label, ?11),
                 translation_role  = COALESCE(translation_role, ?12),
                 source_url        = COALESCE(source_url, ?13),
                 ao3_tags          = COALESCE(ao3_tags, ?14),
                 updated_at        = datetime('now')
             WHERE id = ?1",
            params![
                work_id,
                work.author,
                work.summary,
                work.published_at,
                work.published_prec,
                work.date_source,
                work.completed_at,
                work.word_count,
                work.word_count_source,
                work.language,
                work.language_label,
                work.translation_role,
                work.source_url,
                work.ao3_tags,
            ],
        )?;
        Ok(())
    }

    // ---------------------------------------------------------------- files

    /// Copies `source` into the work's directory and records it.
    ///
    /// The copy is written to a temporary name and moved into place, so an interrupted
    /// import cannot leave a half-written file that later looks like a valid copy.
    ///
    /// A work can hold several versions of the same format — a serialised work gets updated,
    /// a finished work is revised and reposted. `policy` decides which happens, and it is a
    /// per-import choice because the right answer differs by file: usually the older copy is
    /// worth keeping, but sometimes the user only wants the latest.
    pub fn store_file(
        &self,
        work_id: i64,
        source: &Path,
        format: &FileFormat,
        word_count: Option<i64>,
        is_estimated: bool,
        policy: FileVersionPolicy,
    ) -> Result<StoreOutcome> {
        let bytes = std::fs::read(source).map_err(|e| Error::io(source, e))?;
        let hash = crate::hashing::hash_bytes(&bytes);

        // The same content already stored anywhere in the library is not a new version. The
        // import preview already checks this, but a caller can reach here directly, and
        // writing a second physical copy of identical bytes is never what anyone wants.
        if let Some((owner, file_id)) = self.find_work_by_file_hash(&hash)? {
            return Ok(StoreOutcome::DuplicateOf {
                work_id: owner,
                file_id,
            });
        }

        let original_name = source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_owned();

        // Existing files of this format for this work, newest first.
        let existing: Vec<FileRecord> = self
            .files_for_work(work_id)?
            .into_iter()
            .filter(|f| f.format_detail == format.detail())
            .collect();

        let (rel_path, version) = match policy {
            FileVersionPolicy::ReplaceOnly => {
                // One slot per format: overwrite it and reset to a single version.
                (Self::rel_path_for(work_id, format), 1)
            }
            FileVersionPolicy::KeepBoth => {
                let next_version = existing.iter().map(|f| f.version).max().unwrap_or(0) + 1;
                (
                    Self::versioned_rel_path(work_id, format, next_version),
                    next_version,
                )
            }
        };

        let abs = self.abs_path(&rel_path);
        let dir = abs.parent().expect("work path always has a parent");
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;

        let temp = abs.with_extension("tmp-import");
        std::fs::write(&temp, &bytes).map_err(|e| Error::io(&temp, e))?;
        std::fs::rename(&temp, &abs).map_err(|e| Error::io(&abs, e))?;

        // Demote whatever was current for this format. Done in one statement so the format
        // can never briefly hold two current files, or none.
        self.conn.execute(
            "UPDATE files SET role = 'archived', archived_at = datetime('now')
             WHERE work_id = ?1 AND format_detail = ?2 AND role = 'current'",
            params![work_id, format.detail()],
        )?;

        // A path left behind by an earlier ReplaceOnly import is reused rather than
        // accumulating: the row keyed by it is updated instead of duplicated.
        let same_path: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM files WHERE rel_path = ?1",
                params![rel_path],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(id) = same_path {
            self.conn.execute(
                "UPDATE files SET work_id = ?2, format_class = ?3, format_detail = ?4,
                     original_name = ?5, file_hash = ?6, size_bytes = ?7,
                     word_count = ?8, is_estimated = ?9, role = 'current', version = ?10,
                     archived_at = NULL, imported_at = datetime('now')
                 WHERE id = ?1",
                params![
                    id,
                    work_id,
                    format.class().as_str(),
                    format.detail(),
                    original_name,
                    hash,
                    bytes.len() as i64,
                    word_count,
                    is_estimated as i64,
                    version,
                ],
            )?;
            return Ok(StoreOutcome::Stored(Box::new(self.file_record(id)?)));
        }

        self.conn.execute(
            "INSERT INTO files (work_id, format_class, format_detail, original_name,
                                rel_path, file_hash, size_bytes, word_count, is_estimated,
                                role, version, imported_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'current', ?10, datetime('now'))",
            params![
                work_id,
                format.class().as_str(),
                format.detail(),
                original_name,
                rel_path,
                hash,
                bytes.len() as i64,
                word_count,
                is_estimated as i64,
                version,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        Ok(StoreOutcome::Stored(Box::new(self.file_record(id)?)))
    }

    /// Promotes an archived version back to current, demoting whatever was current.
    ///
    /// Used when the user decides an earlier revision is the one they actually want.
    pub fn set_current_version(&self, file_id: i64) -> Result<()> {
        let (work_id, format_detail) = self
            .conn
            .query_row(
                "SELECT work_id, COALESCE(format_detail, '') FROM files WHERE id = ?1",
                params![file_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::Invalid(format!("no file with id {file_id}")))?;

        self.conn.execute(
            "UPDATE files SET role = 'archived', archived_at = datetime('now')
             WHERE work_id = ?1 AND format_detail = ?2 AND role = 'current'",
            params![work_id, format_detail],
        )?;
        self.conn.execute(
            "UPDATE files SET role = 'current', archived_at = NULL WHERE id = ?1",
            params![file_id],
        )?;
        Ok(())
    }

    /// Deletes one archived version, leaving the rest of the work alone.
    pub fn delete_version(&self, file_id: i64) -> Result<()> {
        let role: String = self
            .conn
            .query_row(
                "SELECT role FROM files WHERE id = ?1",
                params![file_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::Invalid(format!("no file with id {file_id}")))?;
        if role == "current" {
            return Err(Error::Invalid(
                "refusing to delete the current version; promote another version first".to_owned(),
            ));
        }
        self.delete_file(file_id)
    }

    fn file_record(&self, id: i64) -> Result<FileRecord> {
        Ok(self.conn.query_row(
            "SELECT id, work_id, format_class, format_detail, original_name, rel_path,
                    COALESCE(file_hash, ''), COALESCE(size_bytes, 0), word_count, is_estimated,
                    role, version
             FROM files WHERE id = ?1",
            params![id],
            |row| {
                Ok(FileRecord {
                    id: row.get(0)?,
                    work_id: row.get(1)?,
                    format_class: class_from_str(&row.get::<_, String>(2)?),
                    format_detail: row.get(3)?,
                    original_name: row.get(4)?,
                    rel_path: row.get(5)?,
                    file_hash: row.get(6)?,
                    size_bytes: row.get(7)?,
                    word_count: row.get(8)?,
                    is_estimated: row.get::<_, i64>(9)? != 0,
                    role: row.get(10)?,
                    version: row.get(11)?,
                })
            },
        )?)
    }

    /// Every file of a work, current versions first, then newest version first.
    ///
    /// The ordering is what the interface relies on: the file a reader wants to open comes
    /// first, and archived revisions follow in reverse chronological order.
    pub fn files_for_work(&self, work_id: i64) -> Result<Vec<FileRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM files WHERE work_id = ?1
             ORDER BY CASE role WHEN 'current' THEN 0 ELSE 1 END, version DESC, format_detail",
        )?;
        let ids: Vec<i64> = stmt
            .query_map(params![work_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter().map(|id| self.file_record(id)).collect()
    }

    /// Applies an edit to a work. Returns an error when the work does not exist.
    ///
    /// Dates are validated before anything is written, so a bad date cannot leave a work
    /// half-updated with the other fields already saved.
    pub fn apply_work_edit(&self, work_id: i64, edit: &WorkEdit) -> Result<()> {
        let exists: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM works WHERE id = ?1",
            params![work_id],
            |r| r.get(0),
        )?;
        if exists == 0 {
            return Err(Error::Invalid(format!("no work with id {work_id}")));
        }

        let published = match &edit.published {
            Some(d) => d.parse()?,
            None => None,
        };
        let completed = match &edit.completed {
            Some(d) => d.parse()?,
            None => None,
        };

        if let Some(status) = &edit.status {
            if status != "has_file" && status != "record_only" {
                return Err(Error::Invalid(format!("unknown status “{status}”")));
            }
        }

        let title = edit
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty());

        self.conn.execute(
            "UPDATE works SET
                 title              = COALESCE(?2, title),
                 author             = CASE WHEN ?3 IS NULL THEN author     ELSE NULLIF(?3, '') END,
                 summary            = CASE WHEN ?4 IS NULL THEN summary    ELSE NULLIF(?4, '') END,
                 my_comment         = CASE WHEN ?5 IS NULL THEN my_comment ELSE NULLIF(?5, '') END,
                 published_at       = CASE WHEN ?6  IS NULL THEN published_at       ELSE ?7  END,
                 published_prec     = CASE WHEN ?6  IS NULL THEN published_prec     ELSE ?8  END,
                 date_is_approx     = CASE WHEN ?6  IS NULL THEN date_is_approx     ELSE ?9  END,
                 date_source        = CASE WHEN ?6  IS NULL THEN date_source        ELSE 'manual' END,
                 completed_at       = CASE WHEN ?10 IS NULL THEN completed_at       ELSE ?11 END,
                 completed_prec     = CASE WHEN ?10 IS NULL THEN completed_prec     ELSE ?12 END,
                 completed_is_approx= CASE WHEN ?10 IS NULL THEN completed_is_approx ELSE ?13 END,
                 completed_source   = CASE WHEN ?10 IS NULL THEN completed_source   ELSE 'manual' END,
                 status             = COALESCE(?14, status),
                 language           = CASE WHEN ?16 IS NULL THEN language       ELSE NULLIF(?16, '') END,
                 language_label     = CASE WHEN ?17 IS NULL THEN language_label ELSE NULLIF(?17, '') END,
                 word_count         = COALESCE(?15, word_count),
                 word_count_source  = CASE WHEN ?15 IS NULL THEN word_count_source ELSE 'manual' END,
                 updated_at         = datetime('now')
             WHERE id = ?1",
            params![
                work_id,
                title,
                edit.author.as_deref(),
                edit.summary.as_deref(),
                edit.my_comment.as_deref(),
                // Outer Option: is this date being edited at all. Inner value: the new
                // value, where None clears the column. The sentinel keeps the two apart.
                edit.published.as_ref().map(|_| 1),
                published.as_ref().map(|(iso, _, _)| iso.clone()),
                published.as_ref().map(|(_, prec, _)| prec.clone()),
                published.as_ref().map(|(_, _, approx)| *approx as i64),
                edit.completed.as_ref().map(|_| 1),
                completed.as_ref().map(|(iso, _, _)| iso.clone()),
                completed.as_ref().map(|(_, prec, _)| prec.clone()),
                completed.as_ref().map(|(_, _, approx)| *approx as i64),
                edit.status.as_deref(),
                // A negative count is not meaningful; treat it as "no change".
                edit.word_count.filter(|n| *n >= 0),
                edit.language.as_deref().map(str::trim),
                edit.language_label.as_deref().map(str::trim),
            ],
        )?;

        if let Some(tags) = &edit.tags {
            self.set_work_tags(work_id, tags)?;
        }
        Ok(())
    }

    /// Replaces a work's title, author, summary and personal comment.
    ///
    /// A field passed as `None` is left alone, so a form that does not expose it cannot
    /// blank it by omission. An empty string is a deliberate clear and is stored as NULL,
    /// which is how "this work has no summary" is represented.
    pub fn update_work_details(
        &self,
        work_id: i64,
        title: Option<&str>,
        author: Option<&str>,
        summary: Option<&str>,
        my_comment: Option<&str>,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE works SET
                 title      = COALESCE(?2, title),
                 author     = CASE WHEN ?3 IS NULL THEN author     ELSE NULLIF(?3, '') END,
                 summary    = CASE WHEN ?4 IS NULL THEN summary    ELSE NULLIF(?4, '') END,
                 my_comment = CASE WHEN ?5 IS NULL THEN my_comment ELSE NULLIF(?5, '') END,
                 updated_at = datetime('now')
             WHERE id = ?1",
            params![
                work_id,
                title.filter(|t| !t.trim().is_empty()),
                author,
                summary,
                my_comment,
            ],
        )?;
        if changed == 0 {
            return Err(Error::Invalid(format!("no work with id {work_id}")));
        }
        Ok(())
    }

    /// Replaces a work's own tags with `names`, creating any that do not exist yet.
    ///
    /// This is the user's own vocabulary, kept separate from AO3's tags, so the set is
    /// replaced wholesale: the caller sends the intended final state. Tags left on no work
    /// are removed afterwards so the filter list does not accumulate orphans.
    pub fn set_work_tags(&self, work_id: i64, names: &[String]) -> Result<Vec<String>> {
        let exists: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM works WHERE id = ?1",
            params![work_id],
            |r| r.get(0),
        )?;
        if exists == 0 {
            return Err(Error::Invalid(format!("no work with id {work_id}")));
        }

        // Normalise here rather than trusting the caller: duplicates differing only in
        // case, and leading/trailing whitespace, are easy to produce from a tag input.
        let mut wanted: Vec<String> = Vec::new();
        for name in names {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !wanted.iter().any(|t| t.eq_ignore_ascii_case(trimmed)) {
                wanted.push(trimmed.to_owned());
            }
        }
        wanted.sort_by_key(|t| t.to_lowercase());

        self.conn
            .execute("DELETE FROM work_tags WHERE work_id = ?1", params![work_id])?;

        for name in &wanted {
            // Targets the NOCASE unique index, so "Fluff" and "fluff" resolve to the one
            // existing row instead of violating the constraint.
            self.conn.execute(
                "INSERT INTO tags (name) VALUES (?1) ON CONFLICT (name) DO NOTHING",
                params![name],
            )?;
            let tag_id: i64 = self.conn.query_row(
                "SELECT id FROM tags WHERE name = ?1 COLLATE NOCASE",
                params![name],
                |r| r.get(0),
            )?;
            self.conn.execute(
                "INSERT INTO work_tags (work_id, tag_id) VALUES (?1, ?2)
                 ON CONFLICT (work_id, tag_id) DO NOTHING",
                params![work_id, tag_id],
            )?;
        }

        self.prune_orphan_tags()?;
        self.tags_for_work(work_id)
    }

    /// Deletes a work, its files and its directory.
    ///
    /// A work owns its directory (`library/<id>/`), so removing the directory removes exactly
    /// the files that belong to it and touches nothing else. Child rows — files, tags links,
    /// series links, relations — go with it through `ON DELETE CASCADE`.
    ///
    /// The directory is removed *before* the row, so a failure part-way leaves a work that still
    /// lists its files rather than an orphaned directory nothing points at.
    pub fn delete_work(&self, work_id: i64) -> Result<()> {
        let exists: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM works WHERE id = ?1",
            params![work_id],
            |r| r.get(0),
        )?;
        if exists == 0 {
            return Err(Error::Invalid(format!("no work with id {work_id}")));
        }

        let dir = self.work_dir(work_id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        }
        self.conn
            .execute("DELETE FROM works WHERE id = ?1", params![work_id])?;

        // Series and tags are shared vocabulary, so a name left with no works is dropped rather
        // than lingering as an empty suggestion.
        self.prune_orphan_tags()?;
        self.conn.execute(
            "DELETE FROM series WHERE id NOT IN (SELECT DISTINCT series_id FROM works_series)",
            [],
        )?;
        Ok(())
    }

    /// Creates an entry with no file: a work the user wants to record before (or without)
    /// downloading it.
    ///
    /// This is how the library stops depending on having the file. AO3 hides and deletes works,
    /// and a bookmark whose file was never downloaded is exactly the case the application exists
    /// for, so it has to be creatable by hand rather than only as a side effect of an import.
    pub fn insert_record_only_work(&self, title: &str, author: Option<&str>) -> Result<i64> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::Invalid("标题不能为空".to_owned()));
        }
        self.conn.execute(
            "INSERT INTO works (title, author, word_count, status, created_at, updated_at)
             VALUES (?1, ?2, 0, 'record_only', datetime('now'), datetime('now'))",
            params![title, author.map(str::trim).filter(|a| !a.is_empty())],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Tags currently on a work, in display order.
    pub fn tags_for_work(&self, work_id: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.name FROM work_tags wt
             JOIN tags t ON t.id = wt.tag_id
             WHERE wt.work_id = ?1
             ORDER BY t.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![work_id], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Removes tags that no longer belong to any work.
    fn prune_orphan_tags(&self) -> Result<()> {
        self.conn.execute(
            "DELETE FROM tags WHERE id NOT IN (SELECT DISTINCT tag_id FROM work_tags)",
            [],
        )?;
        Ok(())
    }

    /// Marks a suggested translation as not wanted.
    ///
    /// Recorded rather than deleted: the row is what AO3's markup said, and deleting it would
    /// only make the suggestion reappear the next time the original is imported.
    pub fn dismiss_pending(&self, external_id: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE pending_relations SET dismissed = 1, dismissed_at = datetime('now')
             WHERE external_id = ?1",
            params![external_id],
        )?;
        if changed == 0 {
            return Err(Error::Invalid(format!(
                "no pending suggestion for works/{external_id}"
            )));
        }
        Ok(())
    }

    /// Undoes [`Self::dismiss_pending`].
    pub fn restore_pending(&self, external_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE pending_relations SET dismissed = 0, dismissed_at = NULL
             WHERE external_id = ?1",
            params![external_id],
        )?;
        Ok(())
    }

    /// Marks every outstanding suggestion as not wanted.
    ///
    /// Offered because a work can legitimately reference several translations the user has no
    /// interest in, and answering them one at a time is tedious. Still recorded rather than
    /// deleted, so it can be undone.
    pub fn dismiss_all_pending(&self) -> Result<usize> {
        let changed = self.conn.execute(
            "UPDATE pending_relations SET dismissed = 1, dismissed_at = datetime('now')
             WHERE dismissed = 0",
            [],
        )?;
        Ok(changed)
    }

    /// Suggestions the user has dismissed, so they can be restored.
    pub fn dismissed_pending(&self) -> Result<Vec<DismissedPending>> {
        let mut stmt = self.conn.prepare(
            "SELECT external_id, title, author FROM pending_relations
             WHERE dismissed = 1
             ORDER BY title IS NULL, title",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DismissedPending {
                external_id: r.get(0)?,
                title: r.get(1)?,
                author: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Replaces a work's series membership, creating series as needed.
    ///
    /// Sent as the intended complete set, like tags. A series left holding no works is removed,
    /// so the suggestion list does not accumulate names that are no longer used.
    pub fn set_work_series(&self, work_id: i64, memberships: &[SeriesMembership]) -> Result<()> {
        let exists: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM works WHERE id = ?1",
            params![work_id],
            |r| r.get(0),
        )?;
        if exists == 0 {
            return Err(Error::Invalid(format!("no work with id {work_id}")));
        }

        self.conn.execute(
            "DELETE FROM works_series WHERE work_id = ?1",
            params![work_id],
        )?;

        for membership in memberships {
            let name = membership.name.trim();
            if name.is_empty() {
                continue;
            }
            // Targets the NOCASE unique index, so "Big Bang" and "big bang" resolve to one
            // series and the existing spelling is kept rather than replaced.
            self.conn.execute(
                "INSERT INTO series (name, created_at) VALUES (?1, datetime('now'))
                 ON CONFLICT (name) DO NOTHING",
                params![name],
            )?;
            let series_id: i64 = self.conn.query_row(
                "SELECT id FROM series WHERE name = ?1 COLLATE NOCASE",
                params![name],
                |r| r.get(0),
            )?;
            self.conn.execute(
                "INSERT INTO works_series (work_id, series_id, position) VALUES (?1, ?2, ?3)
                 ON CONFLICT (work_id, series_id) DO UPDATE SET position = excluded.position",
                params![work_id, series_id, membership.position],
            )?;
        }

        self.conn.execute(
            "DELETE FROM series WHERE id NOT IN (SELECT DISTINCT series_id FROM works_series)",
            [],
        )?;
        Ok(())
    }

    /// Deletes the file from disk and its row. Used when a work is removed.
    pub fn delete_file(&self, file_id: i64) -> Result<()> {
        let rel: Option<String> = self
            .conn
            .query_row(
                "SELECT rel_path FROM files WHERE id = ?1",
                params![file_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(rel) = rel {
            let abs = self.abs_path(&rel);
            if abs.exists() {
                std::fs::remove_file(&abs).map_err(|e| Error::io(&abs, e))?;
            }
        }
        self.conn
            .execute("DELETE FROM files WHERE id = ?1", params![file_id])?;
        Ok(())
    }
}

/// Keeps an extension safe to use as a filename.
///
/// Extensions come from file content and names, so they are not trusted: a value with a
/// path separator or a `..` could otherwise escape the work directory.
fn sanitize_extension(detail: &str) -> String {
    let cleaned: String = detail
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>()
        .to_ascii_lowercase();
    if cleaned.is_empty() {
        "bin".to_owned()
    } else {
        cleaned
    }
}

fn class_from_str(s: &str) -> FormatClass {
    match s {
        "pdf" => FormatClass::Pdf,
        "html" => FormatClass::Html,
        _ => FormatClass::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("bookmarks-store-tests")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn simple_work(title: &str) -> NewWork {
        NewWork {
            title: title.to_owned(),
            word_count: 100,
            ..Default::default()
        }
    }

    #[test]
    fn rel_path_is_numbered_by_work_id_and_keeps_one_slot_per_format() {
        assert_eq!(
            Library::rel_path_for(1, &FileFormat::Html),
            "000001/work.html"
        );
        assert_eq!(
            Library::rel_path_for(42, &FileFormat::Txt),
            "000042/work.txt"
        );
        // An extra format is a `working class` detail, not a class of its own.
        assert_eq!(
            Library::rel_path_for(7, &FileFormat::Epub),
            "000007/work.epub"
        );
        // Unknown formats keep their extension so nothing collides with `work.html`.
        assert_eq!(
            Library::rel_path_for(7, &FileFormat::Other(Some("azw3".into()))),
            "000007/work.azw3"
        );
    }

    #[test]
    fn extension_is_sanitized_against_path_traversal() {
        // Nothing here may produce a path separator or a parent-directory hop.
        for hostile in ["../../etc/passwd", "html/../x", "..", "/abs", "a b"] {
            let rel = Library::rel_path_for(1, &FileFormat::Other(Some(hostile.to_owned())));
            assert!(!rel.contains(".."), "unsafe path from {hostile:?}: {rel}");
            assert_eq!(
                rel.matches('/').count(),
                1,
                "only the work separator: {rel}"
            );
        }
        assert_eq!(
            Library::rel_path_for(1, &FileFormat::Other(None)),
            "000001/work.bin"
        );
        // Non-ASCII extensions would be stripped entirely and fall back to `bin`.
        assert_eq!(
            Library::rel_path_for(1, &FileFormat::Other(Some("中文".into()))),
            "000001/work.bin"
        );
    }

    #[test]
    fn stores_a_file_under_the_work_directory() {
        let root = temp_root("store");
        let lib = Library::open_in_memory(&root).unwrap();
        let work_id = lib.insert_work(&simple_work("T")).unwrap();

        let src = root.join("source.html");
        std::fs::write(&src, b"<html>content</html>").unwrap();

        let rec = lib
            .store_file(
                work_id,
                &src,
                &FileFormat::Html,
                Some(3),
                false,
                FileVersionPolicy::ReplaceOnly,
            )
            .unwrap()
            .into_file()
            .unwrap();

        assert_eq!(rec.work_id, work_id);
        assert_eq!(rec.rel_path, "000001/work.html");
        assert_eq!(rec.original_name, "source.html");
        assert_eq!(rec.size_bytes, 20);
        assert_eq!(rec.word_count, Some(3));
        assert!(!rec.is_estimated);
        assert!(
            lib.abs_path(&rec.rel_path).exists(),
            "file must exist on disk"
        );
        assert!(!rec.file_hash.is_empty());

        // Copies live inside the root, which is what makes a backup a single copy.
        assert!(lib.abs_path(&rec.rel_path).starts_with(&root));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn several_formats_of_one_work_coexist() {
        let root = temp_root("multiformat");
        let lib = Library::open_in_memory(&root).unwrap();
        let work_id = lib.insert_work(&simple_work("T")).unwrap();

        let html = root.join("a.html");
        let epub = root.join("a.epub");
        std::fs::write(&html, b"<html>x</html>").unwrap();
        std::fs::write(&epub, b"PK\x03\x04 epub").unwrap();

        lib.store_file(
            work_id,
            &html,
            &FileFormat::Html,
            Some(1),
            false,
            FileVersionPolicy::ReplaceOnly,
        )
        .unwrap()
        .into_file()
        .unwrap();
        lib.store_file(
            work_id,
            &epub,
            &FileFormat::Epub,
            Some(1),
            true,
            FileVersionPolicy::ReplaceOnly,
        )
        .unwrap()
        .into_file()
        .unwrap();

        let files = lib.files_for_work(work_id).unwrap();
        assert_eq!(files.len(), 2, "both formats recorded under one work");

        let classes: Vec<&str> = files.iter().map(|f| f.format_class.as_str()).collect();
        assert!(classes.contains(&"html"));
        assert!(
            classes.contains(&"other"),
            "epub belongs to the other class"
        );
        // The concrete format is retained for display as "Other · EPUB".
        assert!(files.iter().any(|f| f.format_detail == "epub"));
    }

    #[test]
    fn reimporting_a_format_replaces_it_instead_of_accumulating() {
        let root = temp_root("replace");
        let lib = Library::open_in_memory(&root).unwrap();
        let work_id = lib.insert_work(&simple_work("T")).unwrap();

        let src = root.join("story.txt");
        std::fs::write(&src, b"first version").unwrap();
        let first = lib
            .store_file(
                work_id,
                &src,
                &FileFormat::Txt,
                Some(2),
                true,
                FileVersionPolicy::ReplaceOnly,
            )
            .unwrap()
            .into_file()
            .unwrap();

        std::fs::write(&src, b"second version, longer").unwrap();
        let second = lib
            .store_file(
                work_id,
                &src,
                &FileFormat::Txt,
                Some(4),
                true,
                FileVersionPolicy::ReplaceOnly,
            )
            .unwrap()
            .into_file()
            .unwrap();

        assert_eq!(first.id, second.id, "one slot per format per work");
        assert_eq!(lib.files_for_work(work_id).unwrap().len(), 1);
        assert_ne!(
            first.file_hash, second.file_hash,
            "hash must track the new content"
        );
        assert_eq!(
            std::fs::read(lib.abs_path(&second.rel_path)).unwrap(),
            b"second version, longer"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_temporary_files_are_left_behind() {
        let root = temp_root("tmpclean");
        let lib = Library::open_in_memory(&root).unwrap();
        let work_id = lib.insert_work(&simple_work("T")).unwrap();
        let src = root.join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        lib.store_file(
            work_id,
            &src,
            &FileFormat::Txt,
            None,
            true,
            FileVersionPolicy::ReplaceOnly,
        )
        .unwrap()
        .into_file()
        .unwrap();

        let dir = lib.work_dir(work_id);
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["work.txt".to_string()],
            "only the final file: {names:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hashes_allow_recognising_the_same_file_imported_twice() {
        let root = temp_root("dedupe");
        let lib = Library::open_in_memory(&root).unwrap();
        let work_id = lib.insert_work(&simple_work("T")).unwrap();

        let a = root.join("copy-one.txt");
        let b = root.join("renamed-copy.txt");
        std::fs::write(&a, b"identical bytes").unwrap();
        std::fs::write(&b, b"identical bytes").unwrap();

        let rec = lib
            .store_file(
                work_id,
                &a,
                &FileFormat::Txt,
                None,
                true,
                FileVersionPolicy::ReplaceOnly,
            )
            .unwrap()
            .into_file()
            .unwrap();
        let found = lib.find_work_by_file_hash(&rec.file_hash).unwrap();
        assert_eq!(found, Some((work_id, rec.id)));

        // A different file must not be mistaken for it.
        assert!(lib.find_work_by_file_hash("deadbeef").unwrap().is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn external_id_lookup_finds_the_work() {
        let root = temp_root("extid");
        let lib = Library::open_in_memory(&root).unwrap();
        let work = NewWork {
            external_id: Some("7929115".into()),
            ..simple_work("One, two, three")
        };
        let id = lib.insert_work(&work).unwrap();

        assert_eq!(lib.find_work_by_external_id("7929115").unwrap(), Some(id));
        assert_eq!(lib.find_work_by_external_id("999").unwrap(), None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn filling_missing_metadata_completes_without_overwriting() {
        let root = temp_root("fill");
        let lib = Library::open_in_memory(&root).unwrap();

        // The first import knows only the title, as a bare TXT would.
        let id = lib
            .insert_work(&NewWork {
                title: "T".into(),
                word_count: 0,
                ..Default::default()
            })
            .unwrap();

        // A later HTML import supplies author, summary, language and a word count.
        lib.fill_missing_metadata(
            id,
            &NewWork {
                title: "T".into(),
                author: Some("Someone".into()),
                summary: Some("A summary".into()),
                language: Some("en".into()),
                word_count: 67851,
                word_count_source: Some("ao3".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let (author, summary, lang, words, source): (Option<String>, Option<String>, Option<String>, i64, Option<String>) =
            lib.conn()
                .query_row(
                    "SELECT author, summary, language, word_count, word_count_source FROM works WHERE id = ?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .unwrap();
        assert_eq!(author.as_deref(), Some("Someone"));
        assert_eq!(summary.as_deref(), Some("A summary"));
        assert_eq!(lang.as_deref(), Some("en"));
        assert_eq!(words, 67851);
        assert_eq!(source.as_deref(), Some("ao3"));

        // A user edit must survive a re-import: existing values are never replaced.
        lib.conn()
            .execute(
                "UPDATE works SET author = 'My Correction' WHERE id = ?1",
                params![id],
            )
            .unwrap();
        lib.fill_missing_metadata(
            id,
            &NewWork {
                title: "T".into(),
                author: Some("From File".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let author: Option<String> = lib
            .conn()
            .query_row("SELECT author FROM works WHERE id = ?1", params![id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(author.as_deref(), Some("My Correction"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_work_can_be_recorded_without_a_file() {
        let root = temp_root("recordonly");
        let lib = Library::open_in_memory(&root).unwrap();

        let id = lib
            .insert_record_only_work("  Some Story  ", Some("  An Author  "))
            .unwrap();

        let (title, author, status, words): (String, Option<String>, String, i64) = lib
            .conn()
            .query_row(
                "SELECT title, author, status, word_count FROM works WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        // Surrounding whitespace is trimmed, since it is invisible in the form.
        assert_eq!(title, "Some Story");
        assert_eq!(author.as_deref(), Some("An Author"));
        assert_eq!(
            status, "record_only",
            "an entry with no file is record-only"
        );
        assert_eq!(words, 0);

        assert!(
            lib.files_for_work(id).unwrap().is_empty(),
            "it has no files"
        );

        // An empty author is stored as absent rather than as an empty string.
        let id2 = lib
            .insert_record_only_work("No Author", Some("   "))
            .unwrap();
        let author2: Option<String> = lib
            .conn()
            .query_row(
                "SELECT author FROM works WHERE id = ?1",
                params![id2],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(author2, None);

        // A title is mandatory: an entry with no title cannot be found again.
        assert!(lib.insert_record_only_work("   ", None).is_err());

        // It behaves like any other work: editable, taggable, and it takes files later.
        lib.update_work_details(id, None, None, Some("a summary"), None)
            .unwrap();
        lib.set_work_tags(id, &["someday".into()]).unwrap();
        assert_eq!(lib.tags_for_work(id).unwrap(), vec!["someday".to_string()]);

        let src = root.join("found.txt");
        std::fs::write(&src, b"the story at last").unwrap();
        lib.store_file(
            id,
            &src,
            &FileFormat::Txt,
            Some(4),
            true,
            FileVersionPolicy::ReplaceOnly,
        )
        .unwrap();
        assert_eq!(lib.files_for_work(id).unwrap().len(), 1);
        // Adding a file does not by itself flip the status; that is the user's call.
        let status_after: String = lib
            .conn()
            .query_row("SELECT status FROM works WHERE id = ?1", params![id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status_after, "record_only");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn deleting_a_work_removes_its_files_directory_and_links() {
        let root = temp_root("deletework");
        let lib = Library::open_in_memory(&root).unwrap();

        let keep = lib.insert_work(&simple_work("Keep")).unwrap();
        let doomed = lib.insert_work(&simple_work("Doomed")).unwrap();

        // Files of both formats, plus a version, so several paths are involved.
        for (name, fmt) in [
            ("a.html", FileFormat::Html),
            ("a.epub", FileFormat::Epub),
            ("a2.html", FileFormat::Html),
        ] {
            let src = root.join(name);
            std::fs::write(&src, name.as_bytes()).unwrap();
            lib.store_file(
                doomed,
                &src,
                &fmt,
                Some(1),
                true,
                FileVersionPolicy::KeepBoth,
            )
            .unwrap()
            .into_file()
            .unwrap();
        }
        let survivor = root.join("keep.txt");
        std::fs::write(&survivor, b"keep").unwrap();
        let kept_file = lib
            .store_file(
                keep,
                &survivor,
                &FileFormat::Txt,
                Some(1),
                true,
                FileVersionPolicy::ReplaceOnly,
            )
            .unwrap()
            .into_file()
            .unwrap();

        // A tag shared by both works and a series, so the pruning rules are exercised.
        lib.set_work_tags(doomed, &["shared".into(), "only-doomed".into()])
            .unwrap();
        lib.set_work_tags(keep, &["shared".into()]).unwrap();
        lib.set_work_series(
            doomed,
            &[SeriesMembership {
                name: "Solo Series".into(),
                position: Some(1),
            }],
        )
        .unwrap();

        let doomed_dir = lib.work_dir(doomed);
        assert!(doomed_dir.exists());
        assert_eq!(lib.files_for_work(doomed).unwrap().len(), 3);

        lib.delete_work(doomed).unwrap();

        // The row, its files, its tags link and its series link are gone.
        let rows: i64 = lib
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM works WHERE id = ?1",
                params![doomed],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0, "the work row is gone");
        assert!(lib.files_for_work(doomed).unwrap().is_empty());
        assert!(!doomed_dir.exists(), "the directory must be removed");

        // The other work and its file are untouched.
        assert!(lib.abs_path(&kept_file.rel_path).is_file());
        assert_eq!(lib.files_for_work(keep).unwrap().len(), 1);

        // A tag used only by the deleted work is pruned; a shared one survives.
        let tags: Vec<String> = lib
            .conn()
            .prepare("SELECT name FROM tags ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect();
        assert_eq!(tags, vec!["shared".to_string()], "got {tags:?}");

        // A series left with no works is pruned too.
        let series_count: i64 = lib
            .conn()
            .query_row("SELECT COUNT(*) FROM series", [], |r| r.get(0))
            .unwrap();
        assert_eq!(series_count, 0);

        assert!(
            lib.delete_work(doomed).is_err(),
            "deleting twice is an error"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn deleting_a_work_also_removes_its_relations() {
        let root = temp_root("delrelation");
        let lib = Library::open_in_memory(&root).unwrap();
        let a = lib.insert_work(&simple_work("A")).unwrap();
        let b = lib.insert_work(&simple_work("B")).unwrap();
        lib.conn()
            .execute(
                "INSERT INTO work_relations (work_a, work_b, kind, source)
                 VALUES (?1, ?2, 'translation', 'ao3_html')",
                params![a, b],
            )
            .unwrap();

        lib.delete_work(a).unwrap();

        let remaining: i64 = lib
            .conn()
            .query_row("SELECT COUNT(*) FROM work_relations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            remaining, 0,
            "a relation cannot outlive either of its works"
        );
        let partner: i64 = lib
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM works WHERE id = ?1",
                params![b],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(partner, 1, "the partner survives");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn deleting_a_file_removes_it_from_disk_and_the_database() {
        let root = temp_root("delete");
        let lib = Library::open_in_memory(&root).unwrap();
        let work_id = lib.insert_work(&simple_work("T")).unwrap();
        let src = root.join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        let rec = lib
            .store_file(
                work_id,
                &src,
                &FileFormat::Txt,
                None,
                true,
                FileVersionPolicy::ReplaceOnly,
            )
            .unwrap()
            .into_file()
            .unwrap();
        let abs = lib.abs_path(&rec.rel_path);
        assert!(abs.exists());

        lib.delete_file(rec.id).unwrap();
        assert!(!abs.exists(), "file must be gone from disk");
        assert!(lib.files_for_work(work_id).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn opening_a_library_creates_the_file_store_directory() {
        let root = temp_root("create");
        let lib = Library::open_in_memory(&root).unwrap();
        assert!(lib.root().join("library").is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }
}
