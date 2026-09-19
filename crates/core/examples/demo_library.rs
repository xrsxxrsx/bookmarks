//! Builds a throwaway library of entirely fictional works, for documentation screenshots.
//!
//! ```text
//! cargo run --example demo_library -- --library C:\tmp\bookmarks-demo
//! ```
//!
//! It exists so the screenshots in the READMEs do not show a real person's collection. The
//! previous attempt used the author's own library, which put real work titles, author names
//! and the local library path into a public repository — the same mistake as committing the
//! sample downloads.
//!
//! Everything here goes through the same `store_file` / `apply_work_edit` / `set_work_series`
//! calls the application uses, rather than raw SQL, so the result cannot be a library shape
//! the real code would never produce.
//!
//! The works are invented: the titles, authors and tags below belong to nobody. They are
//! chosen to exercise the features the screenshot is meant to show — a translation paired
//! with its original, several formats under one work, two versions of one file, an estimated
//! word count, a series, and a work with no file at all.

use std::path::{Path, PathBuf};

use bookmarks_core::fileformat::FileFormat;
use bookmarks_core::store::{DateEdit, FileVersionPolicy, Library, SeriesMembership, WorkEdit};

/// One file to store under a work. `second_version` stores the same format twice, which is
/// how a revised chapter file looks.
struct FileSpec {
    format: FileFormat,
    ext: &'static str,
    /// Store this one as a second version of the previous file of the same format.
    as_new_version: bool,
}

struct WorkSpec {
    title: &'static str,
    author: &'static str,
    /// AO3 work id. Fictional, but unique — the column is uniquely indexed.
    external_id: &'static str,
    language: Option<&'static str>,
    language_label: Option<&'static str>,
    words: i64,
    /// `None` marks the count as an estimate, shown with a `~`.
    words_are_exact: bool,
    published: &'static str,
    published_approx: bool,
    completed: &'static str,
    summary: &'static str,
    comment: &'static str,
    tags: &'static [&'static str],
    files: Vec<FileSpec>,
    series: Option<(&'static str, i64)>,
    /// `Some(role)` for a work that participates in a translation pair.
    role: Option<&'static str>,
    /// Index into the spec list of the partner work, for a translation pair.
    partner: Option<usize>,
    /// No file: the record exists because the work is bookmarked but not downloaded.
    record_only: bool,
}

fn file(format: FileFormat, ext: &'static str) -> FileSpec {
    FileSpec {
        format,
        ext,
        as_new_version: false,
    }
}

fn revised(format: FileFormat, ext: &'static str) -> FileSpec {
    FileSpec {
        format,
        ext,
        as_new_version: true,
    }
}

fn specs() -> Vec<WorkSpec> {
    vec![
        WorkSpec {
            title: "The Cartographer's Daughter",
            author: "quillandcompass",
            external_id: "100000001",
            language: Some("en"),
            language_label: Some("English"),
            words: 84_210,
            words_are_exact: true,
            published: "2019-03-14",
            published_approx: false,
            completed: "2021-11-02",
            summary: "A mapmaker's apprentice inherits an atlas of places that do not exist \
                      yet, and discovers she is expected to draw them into being.",
            comment: "Reread every winter. The chapter about the drowned city is the best \
                      thing in the fandom.",
            tags: &["慢热", "世界观宏大", "已完结"],
            files: vec![
                file(FileFormat::Html, "html"),
                revised(FileFormat::Html, "html"),
                file(FileFormat::Epub, "epub"),
                file(FileFormat::Pdf, "pdf"),
            ],
            series: Some(("Atlas Sequence", 1)),
            role: Some("original"),
            partner: Some(1),
            record_only: false,
        },
        WorkSpec {
            title: "制图师的女儿",
            author: "纸上山河",
            external_id: "100000002",
            language: Some("zh"),
            language_label: Some("中文-普通话 國語"),
            words: 61_480,
            words_are_exact: true,
            published: "2020-07-21",
            published_approx: false,
            completed: "2022-04-30",
            summary: "制图学徒继承了一本画满「尚不存在之地」的地图册，随后发现，画出它们正是她的职责。",
            comment: "译笔极稳，把原文那种冷静的语气保住了。",
            tags: &["译文", "慢热"],
            files: vec![file(FileFormat::Html, "html"), file(FileFormat::Txt, "txt")],
            series: None,
            role: Some("translation"),
            partner: Some(0),
            record_only: false,
        },
        WorkSpec {
            title: "Salt and Other Small Gods",
            author: "harbourlight",
            external_id: "100000003",
            language: Some("en"),
            language_label: Some("English"),
            words: 12_940,
            words_are_exact: true,
            published: "2023-01-09",
            published_approx: false,
            completed: "2023-02-18",
            summary: "Three short pieces about the gods a fishing town invents and then \
                      forgets, told in reverse order.",
            comment: "",
            tags: &["短篇集", "克苏鲁风味"],
            files: vec![file(FileFormat::Html, "html")],
            series: Some(("Atlas Sequence", 2)),
            role: None,
            partner: None,
            record_only: false,
        },
        WorkSpec {
            title: "A Longer Winter Than Most",
            author: "harbourlight",
            external_id: "100000004",
            language: Some("en"),
            language_label: Some("English"),
            words: 47_365,
            words_are_exact: true,
            published: "2024-06-02",
            published_approx: false,
            completed: "",
            summary: "The sequel nobody asked for, in which the town digs itself out and \
                      finds the sea has moved.",
            comment: "连载追了一半，先收着。",
            tags: &["连载中", "克苏鲁风味"],
            files: vec![file(FileFormat::Html, "html"), file(FileFormat::Epub, "epub")],
            series: Some(("Atlas Sequence", 3)),
            role: None,
            partner: None,
            record_only: false,
        },
        WorkSpec {
            title: "山雀不叫的那个下午",
            author: "青梅煮酒",
            external_id: "100000005",
            language: Some("zh"),
            language_label: Some("中文-普通话 國語"),
            words: 26_318,
            words_are_exact: false,
            published: "2022",
            published_approx: true,
            completed: "",
            summary: "一个关于沉默与和解的故事。没有大事发生，但每一页都在动。",
            comment: "字数是我自己估的，AO3 上那个数字当时没记下来。",
            tags: &["原作向", "治愈"],
            files: vec![file(FileFormat::Epub, "epub")],
            series: None,
            role: None,
            partner: None,
            record_only: false,
        },
        WorkSpec {
            title: "未命名长篇（待补档）",
            author: "青梅煮酒",
            external_id: "100000006",
            language: Some("zh"),
            language_label: Some("中文-普通话 國語"),
            words: 0,
            words_are_exact: true,
            published: "2021-08-30",
            published_approx: false,
            completed: "",
            summary: "作者删文之前没来得及下载，只有链接和笔记。",
            comment: "在 AO3 上已经找不到了。等有缘再补。",
            tags: &["已删除", "待补档"],
            files: vec![],
            series: None,
            role: None,
            partner: None,
            record_only: true,
        },
    ]
}

/// Placeholder bytes, unique per file so the content-hash duplicate check never fires and
/// silently skips a store. Deliberately obvious so nobody mistakes them for real content.
fn write_placeholder(dir: &Path, stem: &str, ext: &str, marker: usize) -> PathBuf {
    let path = dir.join(format!("{stem}.{ext}"));
    let body = format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\
         <title>{stem}</title></head>\n<body>\n\
         <p>Placeholder for the documentation screenshot (#{marker}).\n\
         Not a real work: the demo library is fictional by design.</p>\n\
         </body></html>\n"
    );
    std::fs::write(&path, body).expect("write placeholder");
    path
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut root: Option<PathBuf> = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--library" | "-l" => root = iter.next().map(PathBuf::from),
            other => {
                eprintln!("unexpected argument: {other}");
                std::process::exit(2);
            }
        }
    }
    let Some(root) = root else {
        eprintln!("usage: demo_library --library <dir>");
        std::process::exit(2);
    };

    // Start from nothing, so re-running gives the same result rather than accumulating.
    if root.exists() {
        std::fs::remove_dir_all(&root).expect("clear previous demo library");
    }

    let staging = std::env::temp_dir().join("bookmarks-demo-source");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).expect("create staging dir");

    println!("building demo library at {}", root.display());
    let lib = Library::open(&root).expect("open library");

    let specs = specs();
    let mut ids: Vec<i64> = Vec::new();
    let mut marker = 0usize;

    for spec in &specs {
        let id = lib
            .insert_record_only_work(spec.title, Some(spec.author))
            .expect("insert work");
        ids.push(id);

        // Files. The engine addresses them by work id, so the placeholder names on disk do
        // not matter — the library layout is what the application produces.
        let mut stored_any = false;
        for (index, fspec) in spec.files.iter().enumerate() {
            marker += 1;
            let stem = format!("demo-{}-{}", spec.external_id, index);
            let source = write_placeholder(&staging, &stem, fspec.ext, marker);
            let policy = if fspec.as_new_version {
                FileVersionPolicy::KeepBoth
            } else {
                FileVersionPolicy::ReplaceOnly
            };
            let outcome = lib
                .store_file(id, &source, &fspec.format, None, !spec.words_are_exact, policy)
                .unwrap_or_else(|e| panic!("store {}: {e}", source.display()));
            stored_any = true;
            println!("  #{id} <- {} ({:?})", fspec.ext, outcome_kind(&outcome));
        }

        let edit = WorkEdit {
            title: Some(spec.title.to_owned()),
            author: Some(spec.author.to_owned()),
            summary: Some(spec.summary.to_owned()),
            my_comment: Some(spec.comment.to_owned()),
            published: Some(DateEdit {
                input: spec.published.to_owned(),
                approx: spec.published_approx,
            }),
            // `None` rather than an empty input: this date is not being edited at all, which
            // is different from a field the user emptied.
            completed: if spec.completed.is_empty() {
                None
            } else {
                Some(DateEdit {
                    input: spec.completed.to_owned(),
                    approx: false,
                })
            },
            status: Some(if stored_any { "has_file" } else { "record_only" }.to_owned()),
            language: Some(spec.language.unwrap_or("").to_owned()),
            language_label: Some(spec.language_label.unwrap_or("").to_owned()),
            word_count: Some(spec.words),
            tags: Some(spec.tags.iter().map(|t| (*t).to_owned()).collect()),
        };
        lib.apply_work_edit(id, &edit).expect("apply metadata");

        if let Some((name, position)) = spec.series {
            lib.set_work_series(
                id,
                &[SeriesMembership {
                    name: name.to_owned(),
                    position: Some(position),
                }],
            )
            .expect("set series");
        }
    }

    // Translation pairs, stored symmetrically like the importer does.
    for (index, spec) in specs.iter().enumerate() {
        if let Some(partner) = spec.partner {
            let a = ids[index].min(ids[partner]);
            let b = ids[index].max(ids[partner]);
            lib.conn()
                .execute(
                    "INSERT OR IGNORE INTO work_relations (work_a, work_b, kind, source) \
                     VALUES (?1, ?2, 'translation', 'manual')",
                    rusqlite::params![a, b],
                )
                .expect("link translation pair");
            println!("  linked #{a} <-> #{b}");
        }
    }

    // Word-count provenance. `apply_work_edit` records a typed count as `manual`, and the
    // interface treats both `ao3` and `manual` as exact — deliberately, since a number the
    // user typed is one they know. A demo work whose count is meant to read as a guess
    // therefore has to say `estimated` outright.
    for (index, spec) in specs.iter().enumerate() {
        if !spec.words_are_exact && spec.words > 0 {
            lib.conn()
                .execute(
                    "UPDATE works SET word_count_source = 'estimated' WHERE id = ?1",
                    rusqlite::params![ids[index]],
                )
                .expect("mark the word count as an estimate");
        }
    }

    // Roles live on the work, and the importer sets them when it recognises a pair.
    for (index, spec) in specs.iter().enumerate() {
        if let Some(role) = spec.role {
            lib.conn()
                .execute(
                    "UPDATE works SET translation_role = ?1 WHERE id = ?2",
                    rusqlite::params![role, ids[index]],
                )
                .expect("set translation role");
        }
    }

    let _ = std::fs::remove_dir_all(&staging);

    println!("\n=== finished ===");
    for (index, spec) in specs.iter().enumerate() {
        println!(
            "  #{:<3} {:<34} {:>8} words ({})",
            ids[index],
            spec.title,
            spec.words,
            if spec.words_are_exact { "exact" } else { "estimated" }
        );
    }
    println!("  works: {}", ids.len());
}

fn outcome_kind(outcome: &bookmarks_core::store::StoreOutcome) -> &'static str {
    use bookmarks_core::store::StoreOutcome;
    match outcome {
        StoreOutcome::Stored(_) => "stored",
        StoreOutcome::DuplicateOf { .. } => "duplicate",
    }
}
