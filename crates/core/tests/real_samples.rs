//! Acceptance test for P1, run against three real AO3 downloads.
//!
//! These are genuine files, not fixtures, so this test is the check that the parser
//! contract still matches what AO3 actually emits. Every expected value below was read
//! out of the files rather than assumed, and the expectations cover the cases that the
//! design was built around:
//!
//!   1. `One_two_three.html`  (works/7929115)  — English original, 67,851 words
//!   2. `Er_San.html`         (works/14885858) — its Chinese translation
//!   3. `Zong_Lu_Jian_Dan_Di_Sha.html` (works/91520771) — a Chinese original
//!
//! The headline requirement is that importing the original and the translation pairs
//! them automatically, with no manual linking.
//!
//! Those downloads are **not committed** — they are other people's stories, and putting
//! them in a public repository would republish them. Drop them into the repository root
//! to run this file; without them every test here skips instead of failing, so a fresh
//! clone still gets a green `cargo test`.

use std::path::{Path, PathBuf};

use bookmarks_core::ao3::{self, Ao3Work, RelationKind, RelationOrigin};
use bookmarks_core::date::DatePrecision;
use bookmarks_core::fileformat::{self, FileFormat, FormatClass};

/// Repository root: `crates/core` -> `crates` -> root.
fn sample(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(name)
}

const ORIGINAL: &str = "One_two_three.html";
const TRANSLATION: &str = "Er_San.html";
const CHINESE: &str = "Zong_Lu_Jian_Dan_Di_Sha.html";

fn load(name: &str) -> Ao3Work {
    let path = sample(name);
    assert!(
        path.exists(),
        "sample file is missing: {} — the test using it should have called samples_present!().",
        path.display()
    );
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let html = fileformat::decode_text(&bytes);
    ao3::parse(&html).unwrap_or_else(|| panic!("{name} should parse as an AO3 download"))
}

// --------------------------------------------------------------------- sample guards

/// Skips the calling test when any of its sample files are absent.
///
/// The real AO3 downloads are deliberately not committed — they are other people's
/// stories — so a fresh clone has none of them. Rust has no runtime "ignored" state, so
/// this macro returns early with `Ok(())` to skip instead of failing. The reason is
/// printed, so a skipped run is visible under `cargo test -- --nocapture`.
macro_rules! samples_present {
    ($($name:expr),+ $(,)?) => {
        let missing: Vec<&str> = [$($name),+]
            .into_iter()
            .filter(|n| !sample(n).exists())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            eprintln!(
                "SKIP {}: sample file(s) not present: {}",
                module_path!(),
                missing.join(", ")
            );
            return Ok(());
        }
    };
}

// ---------------------------------------------------------------- original (English)

#[test]
fn english_original_metadata() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL);

    let w = load(ORIGINAL);

    assert_eq!(w.work_id.as_deref(), Some("7929115"));
    assert_eq!(w.title.as_deref(), Some("One, two, three"));
    assert_eq!(w.author.as_deref(), Some("Severus_divides_into_H"));
    assert_eq!(w.language_label.as_deref(), Some("English"));
    assert_eq!(
        w.language.as_deref(),
        Some("en"),
        "label must map to an ISO code"
    );
    assert_eq!(
        w.role(),
        Some("original"),
        "it lists translations, so it is the original"
    );
    // The summary is prose containing markup; it must be extracted as text with
    // paragraphs preserved.
    let summary = w.summary.as_deref().expect("summary should be extracted");
    assert!(
        summary.contains("Hannibal Lecter"),
        "summary content looks wrong: {summary}"
    );
    assert!(
        !summary.contains('<'),
        "summary must not contain markup: {summary}"
    );
    Ok(())
}

#[test]
fn english_original_word_count_comes_from_ao3() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL);

    let w = load(ORIGINAL);
    // AO3 states this number; it is exact and must never be replaced by an estimate.
    assert_eq!(w.stats.words, Some(67_851));
    Ok(())
}

#[test]
fn english_original_dates_are_day_precision() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL);

    let w = load(ORIGINAL);
    let published = w.stats.published.as_ref().expect("published date");
    assert_eq!(published.iso, "2016-09-01");
    assert_eq!(published.precision, DatePrecision::Day);
    assert!(
        !published.is_approx,
        "AO3 states a real date, so it is not an estimate"
    );
    assert_eq!(published.display(), "2016-09-01");

    let completed = w.stats.completed.as_ref().expect("completed date");
    assert_eq!(completed.iso, "2018-06-19");
    assert_eq!(w.stats.chapters.as_deref(), Some("13/13"));
    Ok(())
}

#[test]
fn english_original_tags_are_split_into_the_right_groups() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL);

    let w = load(ORIGINAL);
    assert!(
        w.tags.fandoms.iter().any(|f| f.contains("Hannibal")),
        "fandoms: {:?}",
        w.tags.fandoms
    );
    assert!(!w.tags.rating.is_empty(), "rating must be captured");
    assert!(
        w.tags.additional.len() > 5,
        "freeform tags carry the bulk of AO3 metadata, got {:?}",
        w.tags.additional
    );
    // `all()` backs the "import AO3 tags as my tags" action.
    assert!(w.tags.all().len() >= w.tags.additional.len());
    Ok(())
}

// ------------------------------------------------------------ translation (Chinese)

#[test]
fn chinese_translation_metadata() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(TRANSLATION);

    let w = load(TRANSLATION);

    assert_eq!(w.work_id.as_deref(), Some("14885858"));
    assert_eq!(w.title.as_deref(), Some("一，二，三"));
    assert_eq!(w.author.as_deref(), Some("lisabart"));
    assert_eq!(w.language_label.as_deref(), Some("中文-普通话 國語"));
    assert_eq!(
        w.language.as_deref(),
        Some("zh"),
        "CJK label must map to 'zh'"
    );
    assert_eq!(w.role(), Some("translation"));
    Ok(())
}

#[test]
fn chinese_translation_word_count_and_dates() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(TRANSLATION);

    let w = load(TRANSLATION);
    assert_eq!(w.stats.words, Some(94_736));
    let published = w.stats.published.as_ref().expect("published date");
    assert_eq!(published.iso, "2018-06-09");
    assert_eq!(published.precision, DatePrecision::Day);
    Ok(())
}

#[test]
fn chinese_translation_points_back_at_the_english_original() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(TRANSLATION);

    let w = load(TRANSLATION);
    let back = w
        .related
        .iter()
        .find(|r| r.kind == RelationKind::TranslationOf)
        .expect("the translation states its original");

    assert_eq!(back.work_id.as_deref(), Some("7929115"));
    assert_eq!(back.title.as_deref(), Some("One, two, three"));
    assert_eq!(back.author.as_deref(), Some("Severus_divides_into_H"));
    Ok(())
}

// ----------------------------------------------------------- chinese original (single)

#[test]
fn chinese_original_metadata() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(CHINESE);

    let w = load(CHINESE);

    assert_eq!(w.work_id.as_deref(), Some("91520771"));
    assert_eq!(w.title.as_deref(), Some("【粽驴】简单地杀个人"));
    assert_eq!(w.author.as_deref(), Some("AnnaBabalaCarlton (seeseaEve)"));
    assert_eq!(w.language.as_deref(), Some("zh"));
    assert_eq!(w.role(), None, "this work has no translation relations");
    assert_eq!(w.stats.words, Some(22_210));
    assert_eq!(
        w.stats.published.as_ref().unwrap().iso,
        "2026-08-28",
        "date must be read in full, not just the year"
    );
    assert_eq!(w.stats.completed.as_ref().unwrap().iso, "2026-09-06");
    Ok(())
}

/// This file's `<title>` is `【粽驴】简单地杀个人 - AnnaBabalaCarlton (seeseaEve) - Fandom`.
/// A parser splitting that string would break; the DOM must be used instead.
#[test]
fn title_containing_separators_is_not_truncated() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(CHINESE);

    let w = load(CHINESE);
    assert_eq!(w.title.as_deref(), Some("【粽驴】简单地杀个人"));
    assert!(
        !w.title.as_deref().unwrap().contains("AnnaBabala"),
        "the author must not leak into the title"
    );
    Ok(())
}

/// Prose in this file contains the phrase "inspired by"; the chapter body must never be
/// scanned for relations, or a fake link would appear.
#[test]
fn document_prose_does_not_create_relations() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(CHINESE);

    let w = load(CHINESE);
    assert!(
        w.related.is_empty(),
        "unexpected relations: {:?}",
        w.related
    );
    Ok(())
}

// ------------------------------------------------------------------ the headline: pairing

#[test]
fn original_and_translation_pair_automatically() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL, TRANSLATION);

    let original = load(ORIGINAL);
    let translation = load(TRANSLATION);

    let original_id = original.work_id.clone().unwrap();
    let translation_id = translation.work_id.clone().unwrap();
    let known = vec![original_id.clone(), translation_id.clone()];

    let plan = ao3::plan_relations(&[original, translation], &known);

    assert_eq!(
        plan.resolved.len(),
        1,
        "exactly one pair, not one per file: {plan:?}"
    );
    let pair = &plan.resolved[0];
    assert_eq!(
        pair.from, "7929115",
        "smaller id first, matching CHECK(work_a < work_b)"
    );
    assert_eq!(pair.to, "14885858");
    assert_eq!(pair.kind, RelationKind::TranslationOf);
    assert_eq!(pair.origin, RelationOrigin::Ao3Html);

    // The original also lists a German translation (81444271) that was not imported.
    // It cannot form a pair, so it must remain pending rather than be dropped.
    assert_eq!(
        plan.pending.len(),
        1,
        "the un-imported German translation: {plan:?}"
    );
    assert_eq!(plan.pending[0].external_id, "81444271");
    Ok(())
}

/// The library's own files do not exist yet when the first one is imported, so the
/// missing partner must be remembered rather than silently dropped.
#[test]
fn importing_only_the_original_leaves_the_translation_pending() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL);

    let original = load(ORIGINAL);
    let plan = ao3::plan_relations(&[original], &["7929115".to_string()]);

    assert!(plan.resolved.is_empty());
    let ids: Vec<&str> = plan
        .pending
        .iter()
        .map(|p| p.external_id.as_str())
        .collect();
    assert!(
        ids.contains(&"14885858"),
        "the Chinese translation: {ids:?}"
    );
    assert!(ids.contains(&"81444271"), "the German translation: {ids:?}");

    let zh = plan
        .pending
        .iter()
        .find(|p| p.external_id == "14885858")
        .unwrap();
    assert_eq!(zh.title.as_deref(), Some("一，二，三"));
    assert_eq!(zh.author.as_deref(), Some("lisabart"));
    assert_eq!(
        zh.language.as_deref(),
        Some("zh"),
        "lang attribute supplies the code"
    );
    assert_eq!(zh.from_work_id, "7929115");
    Ok(())
}

/// Importing the translation alone must also find the original, because the
/// "A translation of" link names it directly.
#[test]
fn importing_only_the_translation_still_records_the_pair() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(TRANSLATION);

    let translation = load(TRANSLATION);
    let plan = ao3::plan_relations(&[translation], &["14885858".to_string()]);

    assert_eq!(plan.resolved.len(), 1);
    assert_eq!(plan.resolved[0].from, "7929115");
    assert_eq!(plan.resolved[0].to, "14885858");
    Ok(())
}

/// End-to-end shape of the workflow: two files dropped in one go, paired with no input
/// from the user, and neither of the unrelated works dragged in.
#[test]
fn batch_import_pairs_the_two_and_leaves_the_third_alone() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL, TRANSLATION, CHINESE);

    let works = vec![load(ORIGINAL), load(TRANSLATION), load(CHINESE)];
    let known: Vec<String> = works.iter().filter_map(|w| w.work_id.clone()).collect();
    assert_eq!(known.len(), 3, "all three declare a work id");

    let plan = ao3::plan_relations(&works, &known);

    assert_eq!(
        plan.resolved.len(),
        1,
        "only the original/translation pair: {plan:?}"
    );
    assert_eq!(plan.resolved[0].from, "7929115");
    assert_eq!(plan.resolved[0].to, "14885858");

    // 91520771 has no relations at all, so it must appear in neither list.
    let mentions_unrelated = plan
        .resolved
        .iter()
        .any(|r| r.from == "91520771" || r.to == "91520771")
        || plan
            .pending
            .iter()
            .any(|p| p.external_id == "91520771" || p.from_work_id == "91520771");
    assert!(!mentions_unrelated, "an unrelated work must not be linked");
    Ok(())
}

// ----------------------------------------------------------------- format classification

#[test]
fn samples_are_classified_as_html_with_ao3_structure_detected() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL, TRANSLATION, CHINESE);

    for name in [ORIGINAL, TRANSLATION, CHINESE] {
        let path = sample(name);
        let bytes = std::fs::read(&path).unwrap();
        let format = fileformat::detect(&path, &bytes);
        assert_eq!(format, FileFormat::Html, "{name} should be HTML");
        assert_eq!(format.class(), FormatClass::Html);
        assert_eq!(format.detail(), "html");
        assert!(format.is_text(), "HTML must contribute searchable text");
    }
    Ok(())
}

/// The full extraction entry point, including the confidence gate that stops a generic
/// HTML page from being imported as a half-empty AO3 record.
#[test]
fn extract_recognises_real_downloads_but_not_lookalike_html() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL, TRANSLATION, CHINESE);

    for name in [ORIGINAL, TRANSLATION, CHINESE] {
        match bookmarks_core::extract(&sample(name)).unwrap() {
            bookmarks_core::Extracted::Ao3(w) => {
                assert!(w.is_confident(), "{name} must parse confidently");
                assert!(w.work_id.is_some() && w.title.is_some());
            }
            other => panic!("{name} should be recognised as an AO3 download, got {other:?}"),
        }
    }

    // Plain HTML that merely mentions AO3 must fall through to the generic path.
    let dir = std::env::temp_dir().join("bookmarks-core-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let lookalike = dir.join("lookalike.html");
    std::fs::write(
        &lookalike,
        "<html><body><p>I read this on archiveofourown.org yesterday.</p></body></html>",
    )
    .unwrap();

    match bookmarks_core::extract(&lookalike).unwrap() {
        bookmarks_core::Extracted::Generic { format, text } => {
            assert_eq!(format, FileFormat::Html);
            assert!(text.is_some());
        }
        other => panic!("a lookalike must not be imported as AO3, got {other:?}"),
    }
    let _ = std::fs::remove_file(&lookalike);
    Ok(())
}

// ------------------------------------------------------------------- search integration

/// The folding rules and the extracted metadata have to work together: the whole point
/// is finding a work from a query that does not match the title byte-for-byte.
#[test]
fn search_finds_the_samples_through_realistic_queries() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL, TRANSLATION, CHINESE);

    let works = [load(ORIGINAL), load(TRANSLATION), load(CHINESE)];

    let index: Vec<(String, &Ao3Work)> = works
        .iter()
        .map(|w| {
            let haystack = bookmarks_core::text::normalize(
                &[
                    w.title.clone().unwrap_or_default(),
                    w.author.clone().unwrap_or_default(),
                    w.summary.clone().unwrap_or_default(),
                    w.tags.all().join(" "),
                ]
                .join(" \u{0} "),
            );
            (haystack, w)
        })
        .collect();

    let search = |query: &str| -> Vec<String> {
        index
            .iter()
            .filter(|(h, _)| bookmarks_core::text::matches(h, query))
            .map(|(_, w)| w.title.clone().unwrap_or_default())
            .collect()
    };

    // Punctuation-insensitive CJK: the title is "一，二，三" but the query has no commas.
    let hits = search("一二三");
    assert_eq!(
        hits,
        vec!["一，二，三".to_string()],
        "CJK query with punctuation folded"
    );

    // Case- and comma-insensitive English.
    assert!(search("ONE TWO").contains(&"One, two, three".to_string()));

    // A character inside a longer CJK title.
    assert!(search("简单地杀").contains(&"【粽驴】简单地杀个人".to_string()));

    // Author search.
    assert!(search("lisabart").contains(&"一，二，三".to_string()));

    // Multi-term across fields: a Chinese query plus an English author name.
    assert!(search("lisabart 一").contains(&"一，二，三".to_string()));

    // A query matching nothing must not return everything.
    assert!(search("zzzznotpresent").is_empty());
    Ok(())
}
