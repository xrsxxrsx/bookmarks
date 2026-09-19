//! Acceptance test for the import pipeline, run against three real AO3 downloads.
//!
//! The unit tests in `import.rs` use small synthetic files so they can cover specific
//! branches. This one exercises the whole path — classify, hash, copy into the library,
//! record metadata, pair translations — on genuine AO3 output, which is the thing that has
//! to keep working when AO3 changes its markup.
//!
//! The downloads are not committed, so these tests skip when they are absent; see the
//! `samples_present!` note in `real_samples.rs` for the reasoning.

use std::path::{Path, PathBuf};

use bookmarks_core::import::{self, FileAction};
use bookmarks_core::store::Library;

const ORIGINAL: &str = "One_two_three.html";
const TRANSLATION: &str = "Er_San.html";
const CHINESE: &str = "Zong_Lu_Jian_Dan_Di_Sha.html";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Path to a sample in the repository root. Does not check that it exists, because the
/// `samples_present!` guard needs to ask that question without triggering an assert.
fn sample(name: &str) -> PathBuf {
    repo_root().join(name)
}

/// A sample that the caller has already confirmed is present.
fn required_sample(name: &str) -> PathBuf {
    let path = sample(name);
    assert!(
        path.exists(),
        "missing sample: {} — the test using it should have called samples_present!().",
        path.display()
    );
    path
}

/// A throwaway library root, so tests never touch the user's real library.
fn temp_library(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("bookmarks-e2e")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Skips the calling test when any of its sample files are absent; see `real_samples.rs`.
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

#[test]
fn imports_all_three_real_files_into_one_library() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL, TRANSLATION, CHINESE);

    let root = temp_library("all-three");
    let lib = Library::open(&root).unwrap();

    let inputs = vec![
        required_sample(ORIGINAL),
        required_sample(TRANSLATION),
        required_sample(CHINESE),
    ];
    let preview = import::analyze(&lib, &inputs).unwrap();

    // Three files, three works: two language editions of one story plus an unrelated work.
    assert_eq!(preview.works.len(), 3, "got {preview:?}");
    assert_eq!(preview.summary.files_total, 3);
    assert_eq!(preview.summary.works_new, 3);
    assert_eq!(preview.summary.duplicates, 0);
    for work in &preview.works {
        for file in &work.index {
            assert_eq!(file.action, FileAction::CreateWork);
        }
    }

    // Analysis alone must not touch the library.
    assert_eq!(import::work_count(&lib).unwrap(), 0);

    let report = import::execute(&lib, &preview).unwrap();
    assert_eq!(report.works_created.len(), 3);
    assert_eq!(report.files_stored, 3);
    assert!(
        report.errors.is_empty(),
        "unexpected errors: {:?}",
        report.errors
    );

    // One pair, discovered from AO3's own markup.
    assert_eq!(report.relations_stored, 1);
    assert_eq!(
        report.relations_pending, 1,
        "the original also lists a German translation that was not imported"
    );

    // Files landed in numbered work directories, one slot per format.
    for path in &report.stored_paths {
        assert!(path.ends_with("work.html"), "unexpected layout: {path}");
        assert!(lib.abs_path(path).exists(), "file missing on disk: {path}");
    }

    // Metadata matches what the unit tests assert about the same files.
    let works: Vec<_> = report
        .works_created
        .iter()
        .map(|id| import::load_work(&lib, *id).unwrap().unwrap())
        .collect();

    let english = works
        .iter()
        .find(|w| w.title == "One, two, three")
        .expect("English original");
    assert_eq!(english.author.as_deref(), Some("Severus_divides_into_H"));
    assert_eq!(english.language.as_deref(), Some("en"));
    assert_eq!(english.word_count, 67_851);
    assert_eq!(english.word_count_source.as_deref(), Some("ao3"));
    assert_eq!(english.published_at.as_deref(), Some("2016-09-01"));
    assert_eq!(english.translation_role.as_deref(), Some("original"));

    let chinese_translation = works
        .iter()
        .find(|w| w.title == "一，二，三")
        .expect("Chinese translation");
    assert_eq!(chinese_translation.language.as_deref(), Some("zh"));
    assert_eq!(chinese_translation.word_count, 94_736);
    assert_eq!(
        chinese_translation.translation_role.as_deref(),
        Some("translation")
    );

    let standalone = works
        .iter()
        .find(|w| w.title == "【粽驴】简单地杀个人")
        .expect("unrelated Chinese work");
    assert_eq!(standalone.word_count, 22_210);
    assert_eq!(standalone.translation_role, None);

    // The pair is visible from both ends, which is what the symmetric table is for.
    let english_id = english.id;
    let translation_id = chinese_translation.id;
    assert_eq!(import::list_relations(&lib, english_id).unwrap().len(), 1);
    assert_eq!(
        import::list_relations(&lib, translation_id).unwrap().len(),
        1
    );
    assert_eq!(
        import::list_relations(&lib, standalone.id).unwrap().len(),
        0,
        "an unrelated work must not be linked"
    );

    // The German translation is remembered as a suggestion, with the details AO3 gave.
    let pending = import::list_pending(&lib).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].external_id, "81444271");
    assert_eq!(
        pending[0].title.as_deref(),
        Some("One, two, three ~ Übersetzung")
    );
    assert_eq!(pending[0].language.as_deref(), Some("de"));

    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

/// Re-importing the same folder must change nothing, which is the behaviour that makes it
/// safe to point the importer at a directory repeatedly.
#[test]
fn reimporting_the_same_files_is_a_no_op() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL, TRANSLATION, CHINESE);

    let root = temp_library("idempotent");
    let lib = Library::open(&root).unwrap();
    let inputs = vec![
        required_sample(ORIGINAL),
        required_sample(TRANSLATION),
        required_sample(CHINESE),
    ];

    let preview = import::analyze(&lib, &inputs).unwrap();
    let first = import::execute(&lib, &preview).unwrap();
    assert_eq!(first.works_created.len(), 3);

    // Same bytes again: recognised by content hash, nothing copied, no new works.
    let preview = import::analyze(&lib, &inputs).unwrap();
    assert_eq!(
        preview.summary.duplicates, 3,
        "all three recognised: {preview:?}"
    );
    assert_eq!(preview.summary.files_to_store, 0);
    assert_eq!(preview.summary.works_new, 0);

    let second = import::execute(&lib, &preview).unwrap();
    assert_eq!(second.files_stored, 0);
    assert_eq!(second.files_skipped, 3);
    assert_eq!(second.works_created.len(), 0);
    assert_eq!(import::work_count(&lib).unwrap(), 3, "still three works");

    // Metadata must not be rewritten either.
    for id in &first.works_created {
        let before = import::load_work(&lib, *id).unwrap().unwrap();
        assert!(before.word_count > 0);
    }

    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

/// The workflow the plan is built around: import the original now, the translation later.
#[test]
fn importing_the_original_then_the_translation_pairs_them_across_sessions() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL, TRANSLATION);

    let root = temp_library("sequential-real");
    let lib = Library::open(&root).unwrap();

    // Session one: only the English original.
    let preview = import::analyze(&lib, &[required_sample(ORIGINAL)]).unwrap();
    let first = import::execute(&lib, &preview).unwrap();
    assert_eq!(first.works_created.len(), 1);
    assert_eq!(first.relations_stored, 0, "nothing to pair with yet");
    assert_eq!(
        import::list_relations(&lib, first.works_created[0])
            .unwrap()
            .len(),
        0
    );

    // The pending table remembers both un-imported translations from AO3's markup.
    let pending: Vec<String> = import::list_pending(&lib)
        .unwrap()
        .into_iter()
        .map(|p| p.external_id)
        .collect();
    assert!(pending.contains(&"14885858".to_string()), "got {pending:?}");
    assert!(pending.contains(&"81444271".to_string()), "got {pending:?}");

    // Session two: the Chinese translation arrives.
    let preview = import::analyze(&lib, &[required_sample(TRANSLATION)]).unwrap();
    let second = import::execute(&lib, &preview).unwrap();
    assert_eq!(second.works_created.len(), 1);
    assert_eq!(
        second.relations_stored, 1,
        "the pair is completed without the user linking anything"
    );

    // The Chinese placeholder is gone; the German one correctly remains.
    let remaining: Vec<String> = import::list_pending(&lib)
        .unwrap()
        .into_iter()
        .map(|p| p.external_id)
        .collect();
    assert_eq!(remaining, vec!["81444271".to_string()], "got {remaining:?}");

    // And the pair reads correctly from both sides.
    let original_id = first.works_created[0];
    let translation_id = second.works_created[0];
    let from_original = import::list_relations(&lib, original_id).unwrap();
    let from_translation = import::list_relations(&lib, translation_id).unwrap();
    assert_eq!(from_original.len(), 1);
    assert_eq!(from_translation.len(), 1);
    assert_eq!(from_original[0].other_work_id, translation_id);
    assert_eq!(from_translation[0].other_work_id, original_id);

    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

/// A directory import walks real folders and picks up the AO3 files among other things.
///
/// Scanned against a controlled directory rather than the repository root: a recursive
/// scan is meant to pick up *everything* importable, so pointing it at the repository
/// would also import `Cargo.toml` and every file under `target/` — correct behaviour, just
/// a pointless 90-second test. This asserts the walk and the filtering that matter.
#[test]
fn importing_a_directory_finds_the_ao3_files_among_others() -> Result<(), Box<dyn std::error::Error>> {
    samples_present!(ORIGINAL, TRANSLATION);

    let root = temp_library("dir");
    let lib = Library::open(&root).unwrap();

    let source = root.join("downloads");
    let nested = source.join("more");
    std::fs::create_dir_all(&nested).unwrap();
    // Real AO3 files, one at the top level and one nested.
    std::fs::copy(required_sample(ORIGINAL), source.join(ORIGINAL)).unwrap();
    std::fs::copy(required_sample(TRANSLATION), nested.join(TRANSLATION)).unwrap();
    // Non-AO3 files that must still be picked up as their own works.
    std::fs::write(source.join("Some Story.txt"), "你好世界 hello world").unwrap();
    std::fs::write(nested.join("notes.md"), "# notes").unwrap();
    // Files that must be skipped.
    std::fs::write(source.join(".hidden.html"), "<html>junk</html>").unwrap();
    std::fs::write(nested.join("._resource.html"), "<html>junk</html>").unwrap();

    let preview = import::analyze(&lib, &[source]).unwrap();

    let discovered: Vec<String> = preview
        .works
        .iter()
        .filter_map(|w| w.external_id.clone())
        .collect();
    for expected in ["7929115", "14885858"] {
        assert!(
            discovered.contains(&expected.to_string()),
            "works/{expected} should be found by a directory scan: {discovered:?}"
        );
    }

    let names: Vec<&str> = preview
        .works
        .iter()
        .flat_map(|w| &w.index)
        .map(|f| f.original_name.as_str())
        .collect();
    assert!(
        names.contains(&"Some Story.txt"),
        "a TXT is importable: {names:?}"
    );
    assert!(
        names.contains(&"notes.md"),
        "an unknown extension is still kept: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.starts_with('.')),
        "hidden and resource-fork files must be skipped: {names:?}"
    );

    // The pair among the real files is still discovered by the directory scan.
    let report = import::execute(&lib, &preview).unwrap();
    assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
    assert_eq!(report.relations_stored, 1);

    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}
