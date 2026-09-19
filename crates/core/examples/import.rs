//! Imports files into a real library on disk and reports what happened.
//!
//! Unlike the test suite, this writes to an actual directory so the layout can be
//! inspected. Point it at a scratch directory to try the pipeline on real downloads:
//!
//! ```text
//! cargo run --example import -- --library C:\tmp\mylib One_two_three.html Er_San.html
//! ```
//!
//! Running it twice is safe and instructive: the second run recognises the files by
//! content hash and copies nothing.

use std::path::PathBuf;

use bookmarks_core::import::{self, FileAction};
use bookmarks_core::store::Library;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut library_root: Option<PathBuf> = None;
    let mut inputs: Vec<PathBuf> = Vec::new();

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--library" | "-l" => library_root = iter.next().map(PathBuf::from),
            "--help" | "-h" => {
                usage();
                return;
            }
            other => inputs.push(PathBuf::from(other)),
        }
    }

    let Some(root) = library_root else {
        usage();
        std::process::exit(2);
    };
    if inputs.is_empty() {
        usage();
        std::process::exit(2);
    }

    println!("library : {}", root.display());
    let lib = match Library::open(&root) {
        Ok(lib) => lib,
        Err(e) => {
            eprintln!("could not open library: {e}");
            std::process::exit(1);
        }
    };

    let preview = match import::analyze(&lib, &inputs) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("could not analyse input: {e}");
            std::process::exit(1);
        }
    };

    println!("\n=== plan ===");
    println!(
        "  {} file(s): {} to store, {} duplicate(s)",
        preview.summary.files_total, preview.summary.files_to_store, preview.summary.duplicates
    );
    println!(
        "  {} work(s): {} new, {} already present",
        preview.summary.works_new + preview.summary.works_existing,
        preview.summary.works_new,
        preview.summary.works_existing
    );

    for work in &preview.works {
        let target = match work.existing_work_id {
            Some(id) => format!("existing work #{id}"),
            None => "new work".to_string(),
        };
        println!(
            "\n  {target}: {:?}{}{}",
            work.title,
            work.author
                .as_deref()
                .map(|a| format!(" by {a}"))
                .unwrap_or_default(),
            work.external_id
                .as_deref()
                .map(|e| format!("  [ao3/{e}]"))
                .unwrap_or_default(),
        );
        for file in &work.index {
            println!(
                "      {:<12} {:<22} {:<24} {}",
                file.format.class().as_str(),
                format!("({})", file.format.detail()),
                match &file.action {
                    FileAction::CreateWork => "create".to_string(),
                    FileAction::AttachToExisting => "attach".to_string(),
                    FileAction::ReplacesStoredFile { .. } => "replace".to_string(),
                    FileAction::KeepBothVersions { .. } => "keep both".to_string(),
                    FileAction::SkippedDuplicate { existing_work_id } =>
                        format!("skip (already in #{existing_work_id})"),
                },
                file.source_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
        }
    }

    if !preview.relations.is_empty() {
        println!("\n  pairs to link:");
        for r in &preview.relations {
            println!(
                "      works/{} <-> works/{}  ({}, {})",
                r.from,
                r.to,
                r.kind.as_str(),
                r.origin.as_str()
            );
        }
    }
    if !preview.pending.is_empty() {
        println!("\n  waiting for a file that is not here yet:");
        for p in &preview.pending {
            println!(
                "      works/{} {:?}{}{}",
                p.external_id,
                p.title.as_deref().unwrap_or("-"),
                p.author
                    .as_deref()
                    .map(|a| format!(" by {a}"))
                    .unwrap_or_default(),
                p.language
                    .as_deref()
                    .map(|l| format!(" [{l}]"))
                    .unwrap_or_default(),
            );
        }
    }

    let report = match import::execute(&lib, &preview) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\nimport failed: {e}");
            std::process::exit(1);
        }
    };

    println!("\n=== result ===");
    println!("  works created : {:?}", report.works_created);
    println!("  works matched : {:?}", report.works_matched);
    println!("  files stored  : {}", report.files_stored);
    println!("  files skipped : {}", report.files_skipped);
    println!("  pairs linked  : {}", report.relations_stored);
    println!("  still waiting : {}", report.relations_pending);
    for path in &report.stored_paths {
        println!("    library/{path}");
    }
    if !report.errors.is_empty() {
        println!("  errors:");
        for (path, err) in &report.errors {
            println!("    {}: {err}", path.display());
        }
    }

    println!("\n=== library contents ===");
    for id in report
        .works_created
        .iter()
        .chain(report.works_matched.iter())
    {
        if let Ok(Some(work)) = import::load_work(&lib, *id) {
            println!(
                "  #{:<4} {:<40} {:>8} words ({})",
                work.id,
                truncate(&work.title, 40),
                work.word_count,
                work.word_count_source.as_deref().unwrap_or("-"),
            );
        }
    }
    match import::work_count(&lib) {
        Ok(n) => println!("  total works: {n}"),
        Err(e) => println!("  total works: unavailable ({e})"),
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn usage() {
    eprintln!("usage: import --library <dir> <file-or-directory> [more...]");
    eprintln!();
    eprintln!("  --library, -l <dir>   library root to import into (created if absent)");
    eprintln!();
    eprintln!("Re-running with the same files is safe: identical content is skipped.");
}
