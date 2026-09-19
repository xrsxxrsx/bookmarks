//! Dumps what the importer sees in a file.
//!
//! Useful both as a smoke test on real downloads and as a debugging aid when a file
//! imports with surprising metadata: it prints the parsed fields next to the format
//! classification and the estimated word count, so the difference between "AO3 stated
//! this" and "we guessed this" is visible rather than implied.
//!
//! Usage: cargo run --example inspect -- <file.html> [more files...]

use std::path::PathBuf;

use bookmarks_core::{ao3, date::DateValue, text, Extracted};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: inspect <file> [more files...]");
        std::process::exit(2);
    }

    // Collect the parsed AO3 works so relations can be planned across all of them.
    let mut aos_works = Vec::new();

    for arg in &args {
        let path = PathBuf::from(arg);
        match bookmarks_core::extract(&path) {
            Ok(Extracted::Ao3(work)) => {
                print_ao3(arg, &work);
                // Unbox: relation planning works on plain works.
                aos_works.push(*work);
            }
            Ok(Extracted::Generic { format, text }) => {
                println!("=== {arg} ===");
                println!("  not an AO3 download");
                println!("  class        : {}", format.class().as_str());
                println!("  format       : {}", format.detail());
                if let Some(t) = &text {
                    let ratios = text::ScriptRatios::of(t);
                    println!("  text chars   : {}", t.chars().count());
                    println!("  est. words   : {} (estimate)", text::count_words(t));
                    println!(
                        "  language     : {} (heuristic)",
                        ratios.guess_language().unwrap_or("unknown")
                    );
                }
                println!();
            }
            Err(e) => println!("=== {arg} ===\n  ERROR: {e}\n"),
        }
    }

    // Plan relations across everything that parsed, so pairing is shown end to end.
    if aos_works.len() > 1 {
        let known: Vec<String> = aos_works.iter().filter_map(|w| w.work_id.clone()).collect();
        let plan = ao3::plan_relations(&aos_works, &known);

        println!("=== planned relations across {} files ===", aos_works.len());
        if plan.resolved.is_empty() {
            println!("  (none resolved)");
        }
        for r in &plan.resolved {
            println!(
                "  PAIRED  works/{from} <-> works/{to}  kind={} source={}",
                r.kind.as_str(),
                r.origin.as_str(),
                from = r.from,
                to = r.to
            );
        }
        if plan.pending.is_empty() {
            println!("  (nothing pending)");
        }
        for p in &plan.pending {
            println!(
                "  PENDING works/{} ({}) title={:?} author={:?} lang={:?} — not imported yet",
                p.external_id,
                p.kind.as_str(),
                p.title.as_deref().unwrap_or("-"),
                p.author.as_deref().unwrap_or("-"),
                p.language.as_deref().unwrap_or("-")
            );
        }
        println!();
    }
}

fn print_ao3(name: &str, w: &ao3::Ao3Work) {
    println!("=== {name} ===");
    println!("  AO3 work id  : {}", w.work_id.as_deref().unwrap_or("-"));
    println!("  url          : {}", w.url.as_deref().unwrap_or("-"));
    println!("  title        : {}", w.title.as_deref().unwrap_or("-"));
    println!("  author       : {}", w.author.as_deref().unwrap_or("-"));
    println!(
        "  language     : {} ({})",
        w.language.as_deref().unwrap_or("-"),
        w.language_label.as_deref().unwrap_or("-")
    );
    println!("  role         : {}", w.role().unwrap_or("-"));

    println!(
        "  published    : {}",
        describe_date(w.stats.published.as_ref())
    );
    println!(
        "  completed    : {}",
        describe_date(w.stats.completed.as_ref())
    );
    match w.stats.words {
        // Word counts stated by AO3 are exact; anything derived would be labelled.
        Some(n) => println!("  word count   : {n} (AO3 stated, exact)"),
        None => println!("  word count   : - (AO3 stated none)"),
    }
    println!(
        "  chapters     : {}",
        w.stats.chapters.as_deref().unwrap_or("-")
    );

    let summary = w.summary.as_deref().unwrap_or("-");
    let preview: String = summary.chars().take(120).collect();
    let ellipsis = if summary.chars().count() > 120 {
        "..."
    } else {
        ""
    };
    println!("  summary      : {preview}{ellipsis}");

    for (label, values) in [
        ("rating", &w.tags.rating),
        ("warnings", &w.tags.warnings),
        ("category", &w.tags.category),
        ("fandoms", &w.tags.fandoms),
        ("relations", &w.tags.relationships),
        ("characters", &w.tags.characters),
        ("ao3 tags", &w.tags.additional),
    ] {
        if !values.is_empty() {
            println!("  {label:<12} : {}", values.join(" | "));
        }
    }

    if w.related.is_empty() {
        println!("  related      : (none)");
    }
    for r in &w.related {
        println!(
            "  related      : {} -> works/{} title={:?} author={:?} lang={:?}",
            r.kind.as_str(),
            r.work_id.as_deref().unwrap_or("-"),
            r.title.as_deref().unwrap_or("-"),
            r.author.as_deref().unwrap_or("-"),
            r.language.as_deref().unwrap_or("-")
        );
    }
    println!();
}

fn describe_date(d: Option<&DateValue>) -> String {
    match d {
        None => "-".to_string(),
        Some(d) => format!(
            "{} (precision={}, approx={}, sorts as {})",
            d.display(),
            d.precision.as_str(),
            d.is_approx,
            d.sort_key()
        ),
    }
}
