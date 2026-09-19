//! Read models handed to the interface.
//!
//! These exist so the frontend never assembles a work from several round-trips. A
//! personal library is small, so `LibraryView` returns everything at once and the UI does
//! its own filtering, sorting, grouping and searching in memory. That keeps those
//! behaviours in one place — the place where they change most often — instead of splitting
//! them between SQL and TypeScript.
//!
//! Nothing here is a database row: the field names and shapes are chosen for the UI.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::store::Library;

/// One physical file of a work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileView {
    pub id: i64,
    /// `pdf` | `html` | `other` — the coarse class the UI groups by.
    pub format_class: String,
    /// The real extension, shown alongside the class as "Other · EPUB".
    pub format_detail: String,
    pub original_name: String,
    pub rel_path: String,
    pub size_bytes: i64,
    /// Absent when no reliable count exists (a PDF, say).
    pub word_count: Option<i64>,
    /// True when the count was measured rather than stated by AO3.
    pub is_estimated: bool,
    /// `current` when this is the version to read, `archived` for an earlier revision that was
    /// kept. Exactly one file per (work, format) is current.
    pub role: String,
    pub version: i64,
    /// True while the file is absent from disk, which the interface states rather than
    /// failing only when the user tries to open it.
    pub exists: bool,
}

impl FileView {
    pub fn is_current(&self) -> bool {
        self.role == "current"
    }
}

/// A series a work belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesView {
    pub series_id: i64,
    pub name: String,
    /// Position within the series. Absent when membership is known but the order is not.
    pub position: Option<i64>,
    /// How many works the series holds, so the interface can show "3 / 5".
    pub total: i64,
}

/// Another work related to this one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedWorkView {
    pub work_id: i64,
    pub title: String,
    pub author: Option<String>,
    pub language: Option<String>,
    /// `translation` | `remix` | `related`
    pub kind: String,
    /// `ao3_html` when AO3 stated it, `manual` when the user linked it.
    pub origin: String,
}

/// A relation to a work that has not been imported, shown as a suggestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingView {
    pub external_id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    /// Title of the work that mentioned it, so the suggestion has context.
    pub from_title: String,
}

/// Everything the interface needs about one work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkView {
    pub id: i64,
    pub title: String,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub my_comment: Option<String>,

    /// Normalised ISO date; partial dates are filled to the start of their range.
    pub published_at: Option<String>,
    /// `day` | `month` | `year` — how much of the date is actually known.
    pub published_prec: Option<String>,
    pub date_is_approx: bool,
    /// The date to display, already formatted (`2019`, `~2019`, `2019-06-09`).
    pub published_display: Option<String>,
    pub completed_at: Option<String>,
    /// `day` | `month` | `year` for the completion date, which is separately editable.
    pub completed_prec: Option<String>,
    pub completed_is_approx: bool,
    /// The completion date to display, already formatted.
    pub completed_display: Option<String>,

    pub word_count: i64,
    /// `ao3` | `estimated` | `manual` — whether the count can be trusted as exact.
    pub word_count_source: Option<String>,
    /// The count to display, marked with `~` when it is an estimate.
    pub word_count_display: String,

    pub language: Option<String>,
    pub language_label: Option<String>,
    pub translation_role: Option<String>,

    pub source_url: Option<String>,
    pub external_id: Option<String>,
    /// AO3's own tags, kept apart from the user's own tags.
    pub ao3_tags: Vec<String>,
    /// The user's own tags.
    pub tags: Vec<String>,

    /// `has_file` | `record_only`
    pub status: String,

    pub files: Vec<FileView>,
    pub related: Vec<RelatedWorkView>,
    /// Series this work belongs to, with its position in each.
    pub series: Vec<SeriesView>,

    /// Everything searchable, folded with the same rules used for matching. Built here so
    /// the frontend does not have to duplicate the folding rules.
    pub search_text: String,
}

/// The whole library, plus the counts shown in the status bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryView {
    pub works: Vec<WorkView>,
    pub pending: Vec<PendingView>,
    pub all_tags: Vec<String>,
    pub counts: LibraryCounts,
}

/// Counts are reported for works and files separately: "how many fics do I have" is a
/// different question from "how many files are on disk".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryCounts {
    pub works_total: i64,
    pub works_with_file: i64,
    pub works_record_only: i64,
    pub files_total: i64,
    /// Works that are one language edition of a story that also exists in another.
    pub works_related: i64,
    pub pending_relations: i64,
}

/// Reads the entire library.
pub fn library_view(lib: &Library) -> Result<LibraryView> {
    let works = work_views(lib)?;
    let pending = pending_views(lib)?;
    let all_tags = all_tags(lib)?;

    let works_total = works.len() as i64;
    let works_record_only = works.iter().filter(|w| w.status == "record_only").count() as i64;
    let files_total = works.iter().map(|w| w.files.len() as i64).sum();
    let works_related = works.iter().filter(|w| !w.related.is_empty()).count() as i64;
    let pending_relations = pending.len() as i64;

    Ok(LibraryView {
        works,
        pending,
        all_tags,
        counts: LibraryCounts {
            works_total,
            works_with_file: works_total - works_record_only,
            works_record_only,
            files_total,
            works_related,
            pending_relations,
        },
    })
}

/// Reads every work with its tags, files and relations.
pub fn work_views(lib: &Library) -> Result<Vec<WorkView>> {
    // Tags for all works in one query rather than one per work.
    let mut tags_by_work: std::collections::HashMap<i64, Vec<String>> = Default::default();
    {
        let mut stmt = lib.conn().prepare(
            "SELECT wt.work_id, t.name
             FROM work_tags wt
             JOIN tags t ON t.id = wt.tag_id
             ORDER BY t.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (work_id, name) = row?;
            tags_by_work.entry(work_id).or_default().push(name);
        }
    }

    let mut out = Vec::new();
    let mut stmt = lib.conn().prepare(
        "SELECT id, title, author, summary, my_comment,
                published_at, published_prec, date_is_approx, completed_at,
                completed_prec, completed_is_approx,
                word_count, word_count_source,
                language, language_label, translation_role,
                source_url, external_id, ao3_tags, status
         FROM works
         ORDER BY id",
    )?;

    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, i64>(7)?,
            r.get::<_, Option<String>>(8)?,
            r.get::<_, Option<String>>(9)?,
            r.get::<_, i64>(10)?,
            r.get::<_, i64>(11)?,
            r.get::<_, Option<String>>(12)?,
            r.get::<_, Option<String>>(13)?,
            r.get::<_, Option<String>>(14)?,
            r.get::<_, Option<String>>(15)?,
            r.get::<_, Option<String>>(16)?,
            r.get::<_, Option<String>>(17)?,
            r.get::<_, Option<String>>(18)?,
            r.get::<_, String>(19)?,
        ))
    })?;

    for row in rows {
        let (
            id,
            title,
            author,
            summary,
            my_comment,
            published_at,
            published_prec,
            date_is_approx,
            completed_at,
            completed_prec,
            completed_is_approx,
            word_count,
            word_count_source,
            language,
            language_label,
            translation_role,
            source_url,
            external_id,
            ao3_tags_json,
            status,
        ) = row?;

        let files = file_views(lib, id)?;
        let related = related_views(lib, id)?;
        let series = series_views(lib, id)?;
        let tags = tags_by_work.remove(&id).unwrap_or_default();
        let ao3_tags = parse_string_array(ao3_tags_json.as_deref());

        let published_display = match (&published_at, published_prec.as_deref()) {
            (Some(iso), prec) => Some(display_date(iso, prec, date_is_approx != 0)),
            _ => None,
        };
        let completed_display = match (&completed_at, completed_prec.as_deref()) {
            (Some(iso), prec) => Some(display_date(iso, prec, completed_is_approx != 0)),
            _ => None,
        };
        let is_estimated = word_count_source.as_deref() != Some("ao3")
            && word_count_source.as_deref() != Some("manual");
        let word_count_display = if word_count == 0 {
            "—".to_string()
        } else if is_estimated {
            format!("~{}", group_digits(word_count))
        } else {
            group_digits(word_count)
        };

        let search_text = crate::text::normalize(
            &[
                title.clone(),
                author.clone().unwrap_or_default(),
                summary.clone().unwrap_or_default(),
                my_comment.clone().unwrap_or_default(),
                language_label.clone().unwrap_or_default(),
                tags.join(" "),
                ao3_tags.join(" "),
            ]
            .join(" \u{0} "),
        );

        out.push(WorkView {
            id,
            title,
            author,
            summary,
            my_comment,
            published_at,
            published_prec,
            date_is_approx: date_is_approx != 0,
            published_display,
            completed_at,
            completed_prec,
            completed_is_approx: completed_is_approx != 0,
            completed_display,
            word_count,
            word_count_source,
            word_count_display,
            language,
            language_label,
            translation_role,
            source_url,
            external_id,
            ao3_tags,
            tags,
            status,
            files,
            related,
            series,
            search_text,
        });
    }
    Ok(out)
}

fn file_views(lib: &Library, work_id: i64) -> Result<Vec<FileView>> {
    let mut stmt = lib.conn().prepare(
        "SELECT id, format_class, COALESCE(format_detail, ''), original_name, rel_path,
                COALESCE(size_bytes, 0), word_count, is_estimated, role, version
         FROM files WHERE work_id = ?1
         ORDER BY CASE role WHEN 'current' THEN 0 ELSE 1 END,
                  CASE format_class WHEN 'html' THEN 0 WHEN 'pdf' THEN 1 ELSE 2 END,
                  version DESC, format_detail",
    )?;
    let rows = stmt.query_map(rusqlite::params![work_id], |r| {
        let rel_path: String = r.get(4)?;
        Ok(FileView {
            id: r.get(0)?,
            format_class: r.get(1)?,
            format_detail: r.get(2)?,
            original_name: r.get(3)?,
            exists: lib.abs_path(&rel_path).is_file(),
            rel_path,
            size_bytes: r.get(5)?,
            word_count: r.get(6)?,
            is_estimated: r.get::<_, i64>(7)? != 0,
            role: r.get(8)?,
            version: r.get(9)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Series a work belongs to, with the size of each so the interface can show a position.
fn series_views(lib: &Library, work_id: i64) -> Result<Vec<SeriesView>> {
    let mut stmt = lib.conn().prepare(
        "SELECT s.id, s.name, ws.position,
                (SELECT COUNT(*) FROM works_series x WHERE x.series_id = s.id)
         FROM works_series ws
         JOIN series s ON s.id = ws.series_id
         WHERE ws.work_id = ?1
         ORDER BY s.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map(rusqlite::params![work_id], |r| {
        Ok(SeriesView {
            series_id: r.get(0)?,
            name: r.get(1)?,
            position: r.get(2)?,
            total: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Every series name in the library, for the editor's suggestions.
pub fn all_series(lib: &Library) -> Result<Vec<String>> {
    let mut stmt = lib
        .conn()
        .prepare("SELECT name FROM series ORDER BY name COLLATE NOCASE")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn related_views(lib: &Library, work_id: i64) -> Result<Vec<RelatedWorkView>> {
    // The symmetric table means one query finds the partner from either side.
    let mut stmt = lib.conn().prepare(
        "SELECT w.id, w.title, w.author, w.language, r.kind, r.source
         FROM work_relations r
         JOIN works w ON w.id = CASE WHEN r.work_a = ?1 THEN r.work_b ELSE r.work_a END
         WHERE r.work_a = ?1 OR r.work_b = ?1
         ORDER BY w.published_at IS NULL, w.published_at, w.id",
    )?;
    let rows = stmt.query_map(rusqlite::params![work_id], |r| {
        Ok(RelatedWorkView {
            work_id: r.get(0)?,
            title: r.get(1)?,
            author: r.get(2)?,
            language: r.get(3)?,
            kind: r.get(4)?,
            origin: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn pending_views(lib: &Library) -> Result<Vec<PendingView>> {
    // Dismissed suggestions are excluded here rather than deleted, so the decision can be
    // undone later and re-importing the original does not bring the suggestion back.
    let mut stmt = lib.conn().prepare(
        "SELECT p.external_id, p.title, p.author, p.language, COALESCE(w.title, '')
         FROM pending_relations p
         LEFT JOIN works w ON w.id = p.from_work_id
         WHERE p.dismissed = 0
         ORDER BY p.title IS NULL, p.title",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(PendingView {
            external_id: r.get(0)?,
            title: r.get(1)?,
            author: r.get(2)?,
            language: r.get(3)?,
            from_title: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn all_tags(lib: &Library) -> Result<Vec<String>> {
    let mut stmt = lib
        .conn()
        .prepare("SELECT name FROM tags ORDER BY name COLLATE NOCASE")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}
/// Renders only the date components that are known, marking a guess with `~` to match the
/// convention used for estimated word counts.
fn display_date(iso: &str, precision: Option<&str>, is_approx: bool) -> String {
    let shown = match precision {
        Some("year") => iso.get(..4).unwrap_or(iso),
        Some("month") => iso.get(..7).unwrap_or(iso),
        _ => iso.get(..10).unwrap_or(iso),
    };
    if is_approx {
        format!("~{shown}")
    } else {
        shown.to_string()
    }
}

/// Thousands separators, so a word count can be read at a glance.
fn group_digits(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// Reads a JSON array of strings, tolerating anything unexpected.
///
/// AO3 tag lists are stored as JSON. A malformed value must not make the whole library
/// unreadable, so a parse failure yields no tags rather than an error.
fn parse_string_array(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{FileVersionPolicy, NewWork};

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join("bookmarks-view-tests")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn insert(lib: &Library, work: NewWork) -> i64 {
        lib.insert_work(&work).unwrap()
    }

    #[test]
    fn empty_library_reports_zeroes() {
        let root = temp_root("empty");
        let lib = Library::open_in_memory(&root).unwrap();
        let view = library_view(&lib).unwrap();
        assert!(view.works.is_empty());
        assert_eq!(
            view.counts,
            LibraryCounts {
                works_total: 0,
                works_with_file: 0,
                works_record_only: 0,
                files_total: 0,
                works_related: 0,
                pending_relations: 0,
            }
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn counts_separate_works_from_files() {
        let root = temp_root("counts");
        let lib = Library::open_in_memory(&root).unwrap();

        let id = insert(
            &lib,
            NewWork {
                title: "Alpha".into(),
                word_count: 1000,
                word_count_source: Some("ao3".into()),
                ..Default::default()
            },
        );
        // Two files on one work: the count that matters is one work, not two.
        for (name, fmt) in [
            ("a.html", crate::fileformat::FileFormat::Html),
            ("a.epub", crate::fileformat::FileFormat::Epub),
        ] {
            let src = root.join(name);
            std::fs::write(&src, name.as_bytes()).unwrap();
            lib.store_file(
                id,
                &src,
                &fmt,
                Some(1),
                true,
                FileVersionPolicy::ReplaceOnly,
            )
            .unwrap()
            .into_file()
            .unwrap();
        }

        // A record-only work, as a bookmark with no downloaded file would be.
        lib.conn()
            .execute(
                "INSERT INTO works (title, summary, status, word_count, created_at, updated_at)
                 VALUES ('Only a record', 'no file yet', 'record_only', 0, 'now', 'now')",
                [],
            )
            .unwrap();

        let view = library_view(&lib).unwrap();
        assert_eq!(view.counts.works_total, 2);
        assert_eq!(view.counts.works_with_file, 1);
        assert_eq!(view.counts.works_record_only, 1);
        assert_eq!(
            view.counts.files_total, 2,
            "files counted separately from works"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn word_count_display_marks_estimates_and_separates_thousands() {
        let root = temp_root("words");
        let lib = Library::open_in_memory(&root).unwrap();
        insert(
            &lib,
            NewWork {
                title: "Exact".into(),
                word_count: 67851,
                word_count_source: Some("ao3".into()),
                ..Default::default()
            },
        );
        insert(
            &lib,
            NewWork {
                title: "Guessed".into(),
                word_count: 23159,
                word_count_source: Some("estimated".into()),
                ..Default::default()
            },
        );
        insert(
            &lib,
            NewWork {
                title: "Unknown".into(),
                word_count: 0,
                ..Default::default()
            },
        );

        let view = library_view(&lib).unwrap();
        let by_title = |t: &str| {
            view.works
                .iter()
                .find(|w| w.title == t)
                .unwrap()
                .word_count_display
                .clone()
        };
        assert_eq!(by_title("Exact"), "67,851", "AO3's number is shown plainly");
        assert_eq!(by_title("Guessed"), "~23,159", "an estimate is marked");
        assert_eq!(by_title("Unknown"), "—", "no count is not zero");
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(-1234567), "-1,234,567");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn date_display_shows_only_the_known_precision() {
        let root = temp_root("dates");
        let lib = Library::open_in_memory(&root).unwrap();
        let cases = [
            ("day", "2019-06-09", "2019-06-09"),
            ("month", "2019-06-01", "2019-06"),
            ("year", "2019-01-01", "2019"),
        ];
        for (i, (prec, iso, expected)) in cases.iter().enumerate() {
            insert(
                &lib,
                NewWork {
                    title: format!("D{i}"),
                    published_at: Some((*iso).to_string()),
                    published_prec: Some((*prec).to_string()),
                    ..Default::default()
                },
            );
            let view = library_view(&lib).unwrap();
            let work = view
                .works
                .iter()
                .find(|w| w.title == format!("D{i}"))
                .unwrap();
            assert_eq!(work.published_display.as_deref(), Some(*expected));
        }

        // An estimate keeps the `~`, matching how estimated word counts are marked.
        insert(
            &lib,
            NewWork {
                title: "Approx".into(),
                published_at: Some("2019-01-01".into()),
                published_prec: Some("year".into()),
                date_is_approx: true,
                ..Default::default()
            },
        );
        let view = library_view(&lib).unwrap();
        let work = view.works.iter().find(|w| w.title == "Approx").unwrap();
        assert_eq!(work.published_display.as_deref(), Some("~2019"));
        // The raw value stays a full date so sorting is unaffected by the display form.
        assert_eq!(work.published_at.as_deref(), Some("2019-01-01"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn search_text_covers_the_fields_the_user_expects() {
        let root = temp_root("search");
        let lib = Library::open_in_memory(&root).unwrap();
        let id = insert(
            &lib,
            NewWork {
                title: "One, two, three".into(),
                author: Some("Severus_divides_into_H".into()),
                summary: Some("A Hunger Games AU.".into()),
                my_comment: Some("read this twice".into()),
                language_label: Some("English".into()),
                ao3_tags: Some(r#"["Angst","Slow Burn"]"#.to_string()),
                ..Default::default()
            },
        );
        // A user tag, which must be searchable as well.
        lib.conn()
            .execute("INSERT INTO tags (name) VALUES ('慢热')", [])
            .unwrap();
        let tag_id: i64 = lib
            .conn()
            .query_row("SELECT id FROM tags WHERE name = '慢热'", [], |r| {
                r.get(0)
            })
            .unwrap();
        lib.conn()
            .execute(
                "INSERT INTO work_tags (work_id, tag_id) VALUES (?1, ?2)",
                rusqlite::params![id, tag_id],
            )
            .unwrap();

        let view = library_view(&lib).unwrap();
        let work = &view.works[0];

        // Folding applied: a query with different punctuation still matches.
        for query in [
            "one two three",
            "hunger games",
            "severus",
            "slow burn",
            "慢热",
            "read this twice",
        ] {
            assert!(
                crate::text::matches(&work.search_text, query),
                "search text should match {query:?}: {}",
                work.search_text
            );
        }
        assert!(!crate::text::matches(&work.search_text, "not present"));
        assert_eq!(work.tags, vec!["慢热".to_string()]);
        assert_eq!(
            work.ao3_tags,
            vec!["Angst".to_string(), "Slow Burn".to_string()]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn malformed_ao3_tag_json_does_not_break_the_library() {
        let root = temp_root("badtags");
        let lib = Library::open_in_memory(&root).unwrap();
        insert(
            &lib,
            NewWork {
                title: "Broken".into(),
                ao3_tags: Some("not json at all".into()),
                ..Default::default()
            },
        );
        let view = library_view(&lib).unwrap();
        assert_eq!(view.works.len(), 1, "one bad field must not hide the work");
        assert!(view.works[0].ao3_tags.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pair_is_visible_from_both_works_and_counted() {
        let root = temp_root("pair");
        let lib = Library::open_in_memory(&root).unwrap();
        let a = insert(
            &lib,
            NewWork {
                title: "Original".into(),
                language: Some("en".into()),
                ..Default::default()
            },
        );
        let b = insert(
            &lib,
            NewWork {
                title: "译文".into(),
                language: Some("zh".into()),
                ..Default::default()
            },
        );
        lib.conn()
            .execute(
                "INSERT INTO work_relations (work_a, work_b, kind, source) VALUES (?1, ?2, 'translation', 'ao3_html')",
                rusqlite::params![a, b],
            )
            .unwrap();

        let view = library_view(&lib).unwrap();
        assert_eq!(
            view.counts.works_related, 2,
            "both sides of the pair are related"
        );

        let original = view.works.iter().find(|w| w.id == a).unwrap();
        assert_eq!(original.related.len(), 1);
        assert_eq!(original.related[0].work_id, b);
        assert_eq!(original.related[0].title, "译文");
        assert_eq!(original.related[0].kind, "translation");
        assert_eq!(original.related[0].origin, "ao3_html");

        // The relation must be readable from the translation's side too.
        let translation = view.works.iter().find(|w| w.id == b).unwrap();
        assert_eq!(translation.related[0].work_id, a);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pending_relation_carries_the_context_of_who_mentioned_it() {
        let root = temp_root("pendingview");
        let lib = Library::open_in_memory(&root).unwrap();
        let id = insert(
            &lib,
            NewWork {
                title: "Original".into(),
                external_id: Some("7929115".into()),
                ..Default::default()
            },
        );
        lib.conn()
            .execute(
                "INSERT INTO pending_relations (external_id, kind, language, title, author, from_work_id)
                 VALUES ('81444271', 'translation', 'de', 'Übersetzung', 'Biest1987', ?1)",
                rusqlite::params![id],
            )
            .unwrap();

        let view = library_view(&lib).unwrap();
        assert_eq!(view.counts.pending_relations, 1);
        assert_eq!(view.pending.len(), 1);
        let p = &view.pending[0];
        assert_eq!(p.external_id, "81444271");
        assert_eq!(p.title.as_deref(), Some("Übersetzung"));
        assert_eq!(p.language.as_deref(), Some("de"));
        assert_eq!(
            p.from_title, "Original",
            "the suggestion says which work mentioned it"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn all_tags_lists_the_shared_vocabulary_once() {
        let root = temp_root("alltags");
        let lib = Library::open_in_memory(&root).unwrap();
        let a = insert(
            &lib,
            NewWork {
                title: "A".into(),
                ..Default::default()
            },
        );
        let b = insert(
            &lib,
            NewWork {
                title: "B".into(),
                ..Default::default()
            },
        );
        for name in ["angst", "Fluff"] {
            lib.conn()
                .execute(
                    "INSERT INTO tags (name) VALUES (?1)",
                    rusqlite::params![name],
                )
                .unwrap();
        }
        let fluff: i64 = lib
            .conn()
            .query_row("SELECT id FROM tags WHERE name = 'Fluff'", [], |r| r.get(0))
            .unwrap();
        // The same tag on two works must appear once in the filter list.
        for work in [a, b] {
            lib.conn()
                .execute(
                    "INSERT INTO work_tags (work_id, tag_id) VALUES (?1, ?2)",
                    rusqlite::params![work, fluff],
                )
                .unwrap();
        }

        let view = library_view(&lib).unwrap();
        assert_eq!(
            view.all_tags,
            vec!["angst".to_string(), "Fluff".to_string()]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn files_are_listed_html_first() {
        let root = temp_root("fileorder");
        let lib = Library::open_in_memory(&root).unwrap();
        let id = insert(
            &lib,
            NewWork {
                title: "A".into(),
                ..Default::default()
            },
        );
        for (name, fmt) in [
            ("a.pdf", crate::fileformat::FileFormat::Pdf),
            ("a.epub", crate::fileformat::FileFormat::Epub),
            ("a.html", crate::fileformat::FileFormat::Html),
        ] {
            let src = root.join(name);
            std::fs::write(&src, name.as_bytes()).unwrap();
            lib.store_file(id, &src, &fmt, None, true, FileVersionPolicy::ReplaceOnly)
                .unwrap()
                .into_file()
                .unwrap();
        }

        let view = library_view(&lib).unwrap();
        let files = &view.works[0].files;
        assert_eq!(files.len(), 3);
        // Readable text first: the html is what the user most likely wants to open.
        assert_eq!(files[0].format_class, "html");
        assert_eq!(files[0].format_detail, "html");
        assert_eq!(files[1].format_class, "pdf");
        assert_eq!(files[2].format_class, "other");
        assert_eq!(
            files[2].format_detail, "epub",
            "class and detail are both exposed"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
