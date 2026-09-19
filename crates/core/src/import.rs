//! Importing files into the library.
//!
//! Import runs in two phases, which is what makes it safe to reason about:
//!
//!  1. [`analyze`] inspects the chosen files and proposes a plan. It writes nothing. The
//!     plan says, per file, whether it should become a new work, join an existing one,
//!     replace a file already stored, or be skipped as an identical duplicate — and it
//!     groups the files that belong to one work.
//!  2. [`execute`] carries the plan out.
//!
//! The split exists because grouping is the one step that can be wrong in a way the user
//! cares about: a preview can be shown and corrected before anything is committed, and a
//! plan can be inspected in tests without touching a filesystem.
//!
//! Grouping follows the same key AO3 itself uses — the work id — so several formats of one
//! work land under one record automatically. Files with no work id (a TXT found elsewhere)
//! fall back to a normalised author/title key.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::ao3::{self, Ao3Work, RelationKind, RelationOrigin, Source};
use crate::error::{Error, Result};
use crate::fileformat::{self, FileFormat};
use crate::hashing;
use crate::store::{FileVersionPolicy, Library, NewWork, StoreOutcome};
use crate::text;

/// What the import intends to do with one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileAction {
    /// The work is new; the file comes with it.
    CreateWork,
    /// The work already exists in the library; the file is attached to it.
    AttachToExisting,
    /// The same content is already stored, so this copy adds nothing.
    SkippedDuplicate { existing_work_id: i64 },
    /// A file of this format is already stored for the work and will be replaced.
    ReplacesStoredFile { existing_work_id: i64 },
    /// A file of this format is already stored, and this one is kept *alongside* it: the
    /// stored file becomes an archived version and the new one becomes current.
    ///
    /// The right answer differs per file — usually the earlier revision is worth keeping,
    /// but sometimes only the latest is wanted — so this is a per-file decision rather than
    /// a single global setting.
    KeepBothVersions { existing_work_id: i64 },
}

impl FileAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileAction::CreateWork => "create_work",
            FileAction::AttachToExisting => "attach_to_existing",
            FileAction::SkippedDuplicate { .. } => "skipped_duplicate",
            FileAction::ReplacesStoredFile { .. } => "replaces_stored_file",
            FileAction::KeepBothVersions { .. } => "keep_both_versions",
        }
    }

    /// Whether this file will actually be copied.
    pub fn is_stored(&self) -> bool {
        !matches!(self, FileAction::SkippedDuplicate { .. })
    }

    /// Whether a file of this format is already stored for the work, so a version policy
    /// applies and the interface should offer the choice.
    pub fn needs_version_choice(&self) -> bool {
        matches!(
            self,
            FileAction::ReplacesStoredFile { .. } | FileAction::KeepBothVersions { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewFile {
    pub source_path: PathBuf,
    pub original_name: String,
    pub format: FileFormat,
    pub file_hash: String,
    pub size_bytes: i64,
    pub ao3: Option<Box<Ao3Work>>,
    /// Estimated for non-AO3 files; stated exactly when AO3 supplied it.
    pub word_count: Option<i64>,
    pub word_count_is_estimated: bool,
    pub action: FileAction,
}

impl PreviewFile {
    pub fn display_title(&self) -> &str {
        if let Some(ao3) = &self.ao3 {
            if let Some(t) = &ao3.title {
                return t;
            }
        }
        &self.original_name
    }
}

/// A work the import will create or add files to, with its files grouped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewWork {
    /// Stable identifier within this preview. Existing works use their database id;
    /// new works use an internal grouping key until they are inserted.
    pub key: String,
    /// Set when the work already exists.
    pub existing_work_id: Option<i64>,
    /// Set when the user chose an existing work for these files by hand, rather than the
    /// automatic rules finding one. Recorded so the interface can say the decision was not its
    /// own, and so a later re-plan does not silently discard it.
    pub manual_target: Option<i64>,
    pub title: String,
    pub author: Option<String>,
    pub external_id: Option<String>,
    pub language: Option<String>,
    pub index: Vec<PreviewFile>,
}

impl PreviewWork {
    pub fn is_new(&self) -> bool {
        self.existing_work_id.is_none()
    }

    /// True when every file is already stored, so nothing will be copied for this work.
    ///
    /// Worth surfacing in the interface: the plan is still shown (so the files are visible and
    /// already-stored content is reported rather than hidden), but choosing a different home for
    /// them changes nothing, because there is nothing to write.
    pub fn is_all_duplicates(&self) -> bool {
        !self.index.is_empty()
            && self
                .index
                .iter()
                .all(|f| matches!(f.action, FileAction::SkippedDuplicate { .. }))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSummary {
    pub files_total: usize,
    pub files_to_store: usize,
    pub duplicates: usize,
    pub works_new: usize,
    pub works_existing: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportPreview {
    pub works: Vec<PreviewWork>,
    /// Relations that can be stored immediately.
    pub relations: Vec<ao3::PlannedRelation>,
    /// Relations whose partner is not in the library yet.
    pub pending: Vec<ao3::PendingRelation>,
    pub summary: ImportSummary,
}

impl ImportPreview {
    pub fn is_empty(&self) -> bool {
        self.works.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    pub works_created: Vec<i64>,
    pub works_matched: Vec<i64>,
    pub files_stored: usize,
    pub files_skipped: usize,
    /// `rel_path` of every stored file, so the caller can show what landed where.
    pub stored_paths: Vec<String>,
    /// Relations recorded or confirmed during this import.
    pub relations_stored: usize,
    /// Relations left waiting for the other side.
    pub relations_pending: usize,
    /// Ids of files whose content turned out to be already stored, found only when the
    /// library changed between the preview and the import.
    pub duplicates_in_library: Vec<i64>,
    /// Files that could not be read or parsed, with the reason. A partial import is
    /// reported rather than aborting everything.
    pub errors: Vec<(PathBuf, String)>,
}

/// Walks the given paths, collecting importable files. Directories are searched
/// recursively; hidden files and macOS resource forks are ignored.
pub fn collect_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            walk(path, &mut out)?;
        } else {
            out.push(path.clone());
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name.starts_with("._") {
            continue;
        }
        let file_type = entry.file_type().map_err(|e| Error::io(&path, e))?;
        if file_type.is_dir() {
            walk(&path, out)?;
        } else if file_type.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// How the import should behave when a file of the same format is already stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionPreference {
    /// Keep the earlier revision as an archived version. The default: the usual reason to
    /// re-import is that the work was updated or reposted, and losing the earlier revision
    /// is worse than carrying a second copy.
    KeepBoth,
    /// Overwrite, one file per format.
    Replace,
}

impl VersionPreference {
    fn action_for(self, existing_work_id: i64) -> FileAction {
        match self {
            VersionPreference::KeepBoth => FileAction::KeepBothVersions { existing_work_id },
            VersionPreference::Replace => FileAction::ReplacesStoredFile { existing_work_id },
        }
    }
}

/// Knobs the caller can set for one import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportOptions {
    pub version_preference: VersionPreference,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions {
            version_preference: VersionPreference::KeepBoth,
        }
    }
}

/// A decision the user made by hand in the import preview: these files belong to that work.
///
/// Needed because the automatic rules deliberately refuse to guess. A work downloaded from AO3
/// is stored under its *title*, while the file is named however the download named it, so
/// importing `Zong_Lu_Jian_Dan_Di_Sha.epub` cannot be matched to a work titled
/// `【粽驴】简单地杀个人`. Refusing to guess is the right default — a silent wrong merge damages an
/// existing record — but without a manual route the two copies can never be joined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualAssignment {
    /// The grouping key of the preview work being reassigned.
    pub key: String,
    /// The existing work these files should belong to.
    pub work_id: i64,
}

/// Builds a plan for importing `paths` using the default options.
pub fn analyze(lib: &Library, paths: &[PathBuf]) -> Result<ImportPreview> {
    analyze_with(lib, paths, ImportOptions::default(), &[])
}

/// Builds a plan for importing `paths` into `lib`, honouring the user's manual assignments.
/// Writes nothing.
pub fn analyze_with(
    lib: &Library,
    paths: &[PathBuf],
    options: ImportOptions,
    manual: &[ManualAssignment],
) -> Result<ImportPreview> {
    let files = collect_files(paths)?;

    let mut preview_files: Vec<PreviewFile> = Vec::new();
    let mut errors: Vec<(PathBuf, String)> = Vec::new();

    for path in &files {
        match inspect(lib, path) {
            Ok(Some(pf)) => preview_files.push(pf),
            Ok(None) => {} // same content already queued for import in this batch
            Err(e) => errors.push((path.clone(), e.to_string())),
        }
    }
    if !errors.is_empty() {
        // Report unreadable files by name rather than failing the whole import.
        let detail = errors
            .iter()
            .map(|(p, e)| format!("{}: {e}", p.display()))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(Error::Invalid(format!(
            "could not read {} file(s): {detail}",
            errors.len()
        )));
    }

    let works = group_into_works(lib, preview_files, options, manual)?;
    let summary = summarize(&works);

    // Relation planning needs the AO3 works and the set of ids that will exist after the
    // import, so a pair is only recorded once both sides are present.
    let ao3_works = collect_ao3_works(&works);
    let mut known: Vec<String> = ao3_works.iter().filter_map(|w| w.work_id.clone()).collect();
    for work in &works {
        if let Some(id) = work.external_id.clone() {
            known.push(id);
        }
    }
    known.sort();
    known.dedup();

    // A work assigned by hand already exists, so its AO3 id is not in the batch and its
    // relations are not planned here: it may have been imported with its own translations long
    // ago, and re-planning them from a file that merely joins it would add nothing.
    let plan = ao3::plan_relations(&ao3_works, &known);

    Ok(ImportPreview {
        works,
        relations: plan.resolved,
        pending: plan.pending,
        summary,
    })
}

/// Inspects one file. Returns `Ok(None)` when the same content was already inspected in
/// this run, so a batch containing the same file twice records it once.
fn inspect(lib: &Library, path: &Path) -> Result<Option<PreviewFile>> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    let file_hash = hashing::hash_bytes(&bytes);
    let format = fileformat::detect(path, &bytes);
    let size_bytes = bytes.len() as i64;

    let original_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_owned();

    // Content already in the library: nothing to do beyond reporting it.
    if let Some((existing_work_id, _file_id)) = lib.find_work_by_file_hash(&file_hash)? {
        return Ok(Some(PreviewFile {
            source_path: path.to_path_buf(),
            original_name,
            format,
            file_hash,
            size_bytes,
            ao3: None,
            word_count: None,
            word_count_is_estimated: true,
            action: FileAction::SkippedDuplicate { existing_work_id },
        }));
    }

    let mut ao3: Option<Box<Ao3Work>> = None;
    let mut word_count = None;
    let mut estimated = true;

    if format.is_text() {
        let decoded = fileformat::decode_text(&bytes);
        if format == FileFormat::Html {
            match ao3::parse(&decoded) {
                Some(work) if work.is_confident() => {
                    // AO3's own count is exact; never substitute an estimate for it.
                    word_count = work.stats.words.map(|w| w as i64);
                    estimated = false;
                    ao3 = Some(Box::new(work));
                }
                _ => {
                    let plain = text::html_to_text(&decoded);
                    word_count = Some(text::count_words(&plain) as i64);
                }
            }
        } else {
            // TXT today; EPUB needs its own container reader, so it contributes a file
            // record and an estimated count from the raw markup is deliberately not
            // attempted.
            if format == FileFormat::Txt {
                word_count = Some(text::count_words(&decoded) as i64);
            }
        }
    }

    Ok(Some(PreviewFile {
        source_path: path.to_path_buf(),
        original_name,
        format,
        file_hash,
        size_bytes,
        ao3,
        word_count,
        word_count_is_estimated: estimated,
        action: FileAction::CreateWork, // refined once grouping is known
    }))
}

/// Decides which work each file belongs to, then sets each file's action.
fn group_into_works(
    lib: &Library,
    files: Vec<PreviewFile>,
    options: ImportOptions,
    manual: &[ManualAssignment],
) -> Result<Vec<PreviewWork>> {
    // Preserve input order: the first file mentioning a work defines the display order,
    // so a preview reads in the order the user selected.
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, PreviewWork> = HashMap::new();
    let mut seen_hashes: HashMap<String, String> = HashMap::new();

    let manual_for = |key: &str| manual.iter().find(|m| m.key == key).map(|m| m.work_id);

    for pf in files {
        // A duplicate still needs to be grouped with the work that already holds its
        // content, otherwise it would form a plan entry of its own and be reported as a
        // new work even though nothing is copied.
        if let FileAction::SkippedDuplicate { existing_work_id } = pf.action {
            let key = format!("work:{existing_work_id}");
            if !groups.contains_key(&key) {
                order.push(key.clone());
                groups.insert(
                    key.clone(),
                    new_group(lib, &key, Some(existing_work_id), None, &pf)?,
                );
            }
            groups.get_mut(&key).expect("just inserted").index.push(pf);
            continue;
        }

        // The same bytes appearing twice in one batch: keep the first, drop the repeat.
        if let Some(first_key) = seen_hashes.get(&pf.file_hash) {
            if let Some(group) = groups.get_mut(first_key) {
                group.index.push(pf);
            }
            continue;
        }

        let key = group_key(&pf);
        seen_hashes.insert(pf.file_hash.clone(), key.clone());

        if !groups.contains_key(&key) {
            order.push(key.clone());
            // A manual choice wins over the automatic rules: the user has information the files
            // do not carry, such as which work a differently-named download belongs to.
            let forced = manual_for(&key);
            groups.insert(key.clone(), new_group(lib, &key, forced, forced, &pf)?);
        }
        groups.get_mut(&key).expect("just inserted").index.push(pf);
    }

    let mut works: Vec<PreviewWork> = order
        .into_iter()
        .filter_map(|k| groups.remove(&k))
        .collect();

    // Now that every work is known, decide per file whether it creates, attaches or
    // replaces.
    for work in &mut works {
        let existing = work.existing_work_id;
        let mut taken_formats: Vec<String> = Vec::new();
        if let Some(id) = existing {
            taken_formats = lib
                .files_for_work(id)?
                .into_iter()
                .map(|f| f.format_detail)
                .collect();
        }
        for pf in &mut work.index {
            pf.action = match (existing, &pf.action) {
                // A duplicate keeps its own verdict.
                (_, FileAction::SkippedDuplicate { .. }) => pf.action.clone(),
                (None, _) => FileAction::CreateWork,
                (Some(id), _) => {
                    let detail = pf.format.detail();
                    if taken_formats.contains(&detail) {
                        options.version_preference.action_for(id)
                    } else {
                        FileAction::AttachToExisting
                    }
                }
            };
            if pf.action.is_stored() {
                taken_formats.push(pf.format.detail());
            }
        }
    }

    Ok(works)
}

/// Key identifying the work a file belongs to.
///
/// AO3's work id comes first, because it is the only identifier that survives a renamed
/// file or a retitled work. Without one, a normalised author/title pair is used, so two
/// formats of the same story still group. A file with neither falls back to its filename
/// stem, which groups `story.html` with `story.epub` while guaranteeing that differently
/// named files stay separate rather than being merged on a guess.
fn group_key(pf: &PreviewFile) -> String {
    if let Some(ao3) = &pf.ao3 {
        if let Some(id) = &ao3.work_id {
            return format!("ao3:{id}");
        }
    }
    let stem = text::normalize(&filename_stem(&pf.original_name));
    let author = pf.ao3.as_ref().and_then(|a| a.author.clone());
    match author.as_deref().map(text::normalize) {
        Some(a) if !a.is_empty() => format!("meta:{a}\u{0}{stem}"),
        _ => format!("stem:{stem}"),
    }
}

fn filename_stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_owned()
}

/// Builds a group's identity, deciding whether it joins an existing work.
///
/// `forced_existing` is set when the file's content is already stored, so the work holding it is
/// known exactly. `manual_existing` is set when the user chose a work in the preview, which
/// outranks every automatic rule because the user knows something the files do not. Otherwise
/// the match is by AO3 work id, and failing that by normalised title/author — which is what lets
/// a bare TXT attach to the work whose HTML was imported earlier even though the TXT carries no
/// id.
fn new_group(
    lib: &Library,
    key: &str,
    forced_existing: Option<i64>,
    manual_existing: Option<i64>,
    pf: &PreviewFile,
) -> Result<PreviewWork> {
    let title = match pf.ao3.as_ref().and_then(|a| a.title.clone()) {
        Some(title) => title,
        None => filename_stem(&pf.original_name),
    };
    let author = pf.ao3.as_ref().and_then(|a| a.author.clone());

    let existing_work_id = match (manual_existing, forced_existing) {
        // A manual choice wins outright.
        (Some(id), _) => Some(id),
        (None, Some(id)) => Some(id),
        (None, None) => match pf.ao3.as_ref().and_then(|a| a.work_id.clone()) {
            Some(external_id) => lib.find_work_by_external_id(&external_id)?,
            // No AO3 id: fall back to the normalised title, which still identifies a work
            // already in the library. This never invents a match for a title that is not
            // present, so unrelated files stay separate.
            None => lib.find_work_by_title(&title)?,
        },
    };

    Ok(PreviewWork {
        key: key.to_owned(),
        existing_work_id,
        manual_target: manual_existing,
        title,
        author,
        external_id: pf.ao3.as_ref().and_then(|a| a.work_id.clone()),
        language: pf.ao3.as_ref().and_then(|a| a.language.clone()),
        index: Vec::new(),
    })
}

fn summarize(works: &[PreviewWork]) -> ImportSummary {
    let mut s = ImportSummary::default();
    for work in works {
        if work.is_new() {
            s.works_new += 1;
        } else {
            s.works_existing += 1;
        }
        for f in &work.index {
            s.files_total += 1;
            if f.action.is_stored() {
                s.files_to_store += 1;
            } else {
                s.duplicates += 1;
            }
        }
    }
    s
}

fn collect_ao3_works(works: &[PreviewWork]) -> Vec<Ao3Work> {
    works
        .iter()
        .flat_map(|w| w.index.iter())
        .filter_map(|f| f.ao3.as_deref().cloned())
        .collect()
}

/// Applies a plan produced by [`analyze`].
///
/// Files whose action is `SkippedDuplicate` are not copied. Metadata is written only for
/// works that are created; for an existing work, empty fields are filled in but no stored
/// value is overwritten, so a re-import cannot undo a manual correction.
pub fn execute(lib: &Library, preview: &ImportPreview) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    // Maps a preview key (or AO3 work id) to the real database id, so relations can be
    // recorded against ids that exist.
    let mut id_by_key: HashMap<String, i64> = HashMap::new();
    let mut id_by_external: HashMap<String, i64> = HashMap::new();

    for work in &preview.works {
        let structured = work.index.iter().find_map(|f| f.ao3.clone());

        let new_work = build_new_work(work, structured.as_deref());
        let work_id = match work.existing_work_id {
            Some(id) => {
                lib.fill_missing_metadata(id, &new_work)?;
                report.works_matched.push(id);
                id
            }
            None => {
                let id = lib.insert_work(&new_work)?;
                report.works_created.push(id);
                id
            }
        };
        id_by_key.insert(work.key.clone(), work_id);
        if let Some(ext) = &work.external_id {
            id_by_external.insert(ext.clone(), work_id);
        }

        for pf in &work.index {
            if !pf.action.is_stored() {
                report.files_skipped += 1;
                continue;
            }
            // The version policy rides on the file's own plan entry, so a per-file choice
            // made in the preview reaches the store without a separate argument.
            let policy = match pf.action {
                FileAction::KeepBothVersions { .. } => FileVersionPolicy::KeepBoth,
                _ => FileVersionPolicy::ReplaceOnly,
            };
            match lib.store_file(
                work_id,
                &pf.source_path,
                &pf.format,
                pf.word_count,
                pf.word_count_is_estimated,
                policy,
            ) {
                Ok(StoreOutcome::Stored(record)) => {
                    report.files_stored += 1;
                    report.stored_paths.push(record.rel_path);
                }
                Ok(StoreOutcome::DuplicateOf { file_id, .. }) => {
                    // The bytes turned out to be already stored. The preview normally
                    // catches this, so reaching here means the library changed in between;
                    // skipping is the safe outcome either way.
                    report.files_skipped += 1;
                    report.duplicates_in_library.push(file_id);
                }
                Err(e) => report.errors.push((pf.source_path.clone(), e.to_string())),
            }
        }
    }

    // Now that real ids exist, record the relations and the ones still waiting.
    //
    // A pair can be completed by either path: `store_relations` handles a pair where both
    // works arrived in this same batch, and `resolve_pending_after_import` handles a pair
    // whose other half was imported earlier. Both produce a stored relation, so both count
    // towards `relations_stored` — reporting one of them as merely "pending" would tell the
    // user their translation was not linked when it was.
    let stored_from_batch = store_relations(lib, &preview.relations, &id_by_external)?;
    store_pending(lib, &preview.pending, &id_by_external)?;

    let completed_from_pending = resolve_pending_after_import(lib, &id_by_external)?;
    report.relations_stored = stored_from_batch + completed_from_pending;

    // Anything still in the table is genuinely waiting on a work that has not been
    // imported, which is what the UI shows as a suggestion.
    report.relations_pending = count_pending(lib)?;

    Ok(report)
}

/// How many relations are still waiting for their other half.
fn count_pending(lib: &Library) -> Result<usize> {
    let n: i64 = lib
        .conn()
        .query_row("SELECT COUNT(*) FROM pending_relations", [], |r| r.get(0))?;
    Ok(n as usize)
}

fn build_new_work(work: &PreviewWork, ao3: Option<&Ao3Work>) -> NewWork {
    let stated_word_count = ao3.and_then(|a| a.stats.words).map(|w| w as i64);
    // Prefer the stated count; otherwise take the largest per-file estimate, since an
    // extra format of the same text is still the same text.
    let best_estimate = work
        .index
        .iter()
        .filter(|f| f.action.is_stored())
        .filter_map(|f| f.word_count)
        .max();

    let (word_count, word_count_source) = match (stated_word_count, best_estimate) {
        (Some(n), _) => (n, Some(Source::Ao3)),
        (None, Some(n)) => (n, Some(Source::Estimated)),
        (None, None) => (0, None),
    };

    let date = ao3.and_then(|a| a.stats.published.clone());

    NewWork {
        title: work.title.clone(),
        author: work.author.clone(),
        summary: ao3.and_then(|a| a.summary.clone()),
        my_comment: None,
        published_at: date.as_ref().map(|d| d.iso.clone()),
        published_prec: date.as_ref().map(|d| d.precision.as_str().to_owned()),
        date_is_approx: date.as_ref().is_some_and(|d| d.is_approx),
        date_source: date.as_ref().map(|_| Source::Ao3.as_str().to_owned()),
        completed_at: ao3.and_then(|a| a.stats.completed.as_ref().map(|d| d.iso.clone())),
        word_count,
        word_count_source: word_count_source.map(|s| s.as_str().to_owned()),
        language: work.language.clone(),
        language_label: ao3.and_then(|a| a.language_label.clone()),
        translation_role: ao3.and_then(|a| a.role()).map(str::to_owned),
        source_url: ao3.and_then(|a| a.url.clone()),
        external_id: work.external_id.clone(),
        ao3_tags: ao3
            .map(|a| serde_json::to_string(&a.tags.all()).unwrap_or_else(|_| "[]".to_owned())),
    }
}

/// Stores resolved relations, skipping any that cannot be mapped to two real works.
fn store_relations(
    lib: &Library,
    relations: &[ao3::PlannedRelation],
    id_by_external: &HashMap<String, i64>,
) -> Result<usize> {
    let mut stored = 0;
    for rel in relations {
        let (Some(a), Some(b)) = (
            id_by_external.get(&rel.from).copied(),
            id_by_external.get(&rel.to).copied(),
        ) else {
            continue;
        };
        // Pairs are ordered by id, matching CHECK (work_a < work_b).
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        if lo == hi {
            continue;
        }
        let changed = lib.conn().execute(
            "INSERT INTO work_relations (work_a, work_b, kind, source)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (work_a, work_b) DO NOTHING",
            params![lo, hi, rel.kind.as_str(), rel.origin.as_str()],
        )?;
        stored += changed;
    }
    Ok(stored)
}

/// Records relations whose partner has not been imported.
fn store_pending(
    lib: &Library,
    pending: &[ao3::PendingRelation],
    id_by_external: &HashMap<String, i64>,
) -> Result<()> {
    for p in pending {
        // A partner that arrived in this very batch is resolved elsewhere; not pending.
        if id_by_external.contains_key(&p.external_id) {
            continue;
        }
        let Some(from_id) = id_by_external.get(&p.from_work_id).copied() else {
            continue;
        };
        lib.conn().execute(
            "INSERT INTO pending_relations (external_id, kind, language, title, author, from_work_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (external_id, from_work_id) DO UPDATE SET
                 title = COALESCE(excluded.title, pending_relations.title),
                 author = COALESCE(excluded.author, pending_relations.author),
                 language = COALESCE(excluded.language, pending_relations.language)",
            params![
                p.external_id,
                p.kind.as_str(),
                p.language,
                p.title,
                p.author,
                from_id
            ],
        )?;
    }
    Ok(())
}

/// Completes pairings that were waiting on this import, and clears the placeholders.
///
/// This is what makes the workflow painless in practice: importing the original first
/// records "the Chinese translation exists, id 14885858"; when that file is imported
/// later, the relation appears without the user linking anything.
fn resolve_pending_after_import(
    lib: &Library,
    id_by_external: &HashMap<String, i64>,
) -> Result<usize> {
    if id_by_external.is_empty() {
        return Ok(0);
    }

    let mut completed = 0;
    let mut to_delete: Vec<(String, i64)> = Vec::new();

    for (external_id, new_work_id) in id_by_external {
        let rows: Vec<(i64, String)> = {
            let mut stmt = lib.conn().prepare(
                "SELECT from_work_id, kind FROM pending_relations WHERE external_id = ?1",
            )?;
            let mapped = stmt.query_map(params![external_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?;
            mapped.collect::<std::result::Result<Vec<_>, _>>()?
        };

        for (from_work_id, kind) in rows {
            if from_work_id == *new_work_id {
                to_delete.push((external_id.clone(), from_work_id));
                continue;
            }
            let (lo, hi) = if from_work_id < *new_work_id {
                (from_work_id, *new_work_id)
            } else {
                (*new_work_id, from_work_id)
            };
            let inserted = lib.conn().execute(
                "INSERT INTO work_relations (work_a, work_b, kind, source)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (work_a, work_b) DO NOTHING",
                params![lo, hi, kind, RelationOrigin::Ao3Html.as_str()],
            )?;
            completed += inserted;
            to_delete.push((external_id.clone(), from_work_id));
        }
    }

    for (external_id, from_work_id) in to_delete {
        lib.conn().execute(
            "DELETE FROM pending_relations WHERE external_id = ?1 AND from_work_id = ?2",
            params![external_id, from_work_id],
        )?;
    }
    Ok(completed)
}

/// A relation whose other half has not been imported, for display as a suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEntry {
    /// AO3 work id of the missing partner.
    pub external_id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    /// ISO 639-1 code, when AO3 marked the language on the link.
    pub language: Option<String>,
}

/// Relations currently waiting for the other side, for display as suggestions.
pub fn list_pending(lib: &Library) -> Result<Vec<PendingEntry>> {
    let mut stmt = lib.conn().prepare(
        "SELECT external_id, title, author, language FROM pending_relations
         ORDER BY title IS NULL, title",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(PendingEntry {
            external_id: r.get(0)?,
            title: r.get(1)?,
            author: r.get(2)?,
            language: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// A relation between two works in the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationEntry {
    /// The other work in the pair.
    pub other_work_id: i64,
    pub kind: RelationKind,
    /// `ao3_html` when AO3 stated it, `manual` when the user linked it.
    pub origin: String,
}

/// Relations stored between works, for display.
pub fn list_relations(lib: &Library, work_id: i64) -> Result<Vec<RelationEntry>> {
    let mut stmt = lib.conn().prepare(
        "SELECT work_b, kind, source FROM work_relations WHERE work_a = ?1
         UNION ALL
         SELECT work_a, kind, source FROM work_relations WHERE work_b = ?1",
    )?;
    let rows = stmt.query_map(params![work_id], |r| {
        let kind: String = r.get(1)?;
        Ok(RelationEntry {
            other_work_id: r.get(0)?,
            kind: kind_from_str(&kind),
            origin: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn kind_from_str(s: &str) -> RelationKind {
    match s {
        "remix" => RelationKind::RemixOf,
        "related" => RelationKind::RelatedTo,
        _ => RelationKind::TranslationOf,
    }
}

/// Used by tests and diagnostics: how many works the library holds.
pub fn work_count(lib: &Library) -> Result<i64> {
    Ok(lib
        .conn()
        .query_row("SELECT COUNT(*) FROM works", [], |r| r.get(0))?)
}

/// A work's stored metadata, for verifying an import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredWork {
    pub id: i64,
    pub title: String,
    pub author: Option<String>,
    pub language: Option<String>,
    pub word_count: i64,
    pub word_count_source: Option<String>,
    pub published_at: Option<String>,
    pub published_prec: Option<String>,
    pub translation_role: Option<String>,
    pub status: String,
}

pub fn load_work(lib: &Library, id: i64) -> Result<Option<StoredWork>> {
    Ok(lib
        .conn()
        .query_row(
            "SELECT id, title, author, language, word_count, word_count_source,
                    published_at, published_prec, translation_role, status
             FROM works WHERE id = ?1",
            params![id],
            |r| {
                Ok(StoredWork {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    author: r.get(2)?,
                    language: r.get(3)?,
                    word_count: r.get(4)?,
                    word_count_source: r.get(5)?,
                    published_at: r.get(6)?,
                    published_prec: r.get(7)?,
                    translation_role: r.get(8)?,
                    status: r.get(9)?,
                })
            },
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("bookmarks-import-tests")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    /// Minimal AO3 download with a chosen work id, title and language.
    fn ao3_html(work_id: &str, title: &str, language: &str, words: u64, related: &str) -> String {
        format!(
            r#"<!DOCTYPE html><html><head><title>{title} - AU - F</title></head><body>
<div id="preface">
  <p class="message">Posted on AO3 at <a href="https://archiveofourown.org/works/{work_id}">link</a>.</p>
  <div class="meta">
    <dl class="tags">
      <dt>Rating:</dt><dd><a href="/tags/G">General</a></dd>
      <dt>Additional Tags:</dt><dd><a href="/tags/x">Fluff</a></dd>
      <dt>Language:</dt><dd>{language}</dd>
      <dt>Stats:</dt><dd>Published: 2020-03-04 Words: {words} Chapters: 1/1</dd>
    </dl>
    {related}
    <h1>{title}</h1>
    <div class="byline">by <a rel="author" href="/users/u">Someone</a></div>
    <p>Summary</p>
    <blockquote class="userstuff"><p>A summary.</p></blockquote>
  </div>
</div>
<div id="chapters" class="userstuff"><p>Body text here.</p></div>
</body></html>"#
        )
    }

    #[test]
    fn analyzes_then_imports_a_single_html_file() {
        let root = temp_root("single");
        let lib = Library::open_in_memory(&root).unwrap();
        let src = write(
            &root,
            "story.html",
            &ao3_html("111", "Alpha", "English", 1234, ""),
        );

        let preview = analyze(&lib, std::slice::from_ref(&src)).unwrap();
        assert_eq!(preview.works.len(), 1);
        assert_eq!(preview.works[0].title, "Alpha");
        assert!(preview.works[0].is_new());
        assert_eq!(preview.summary.works_new, 1);
        assert_eq!(preview.summary.files_to_store, 1);
        assert_eq!(preview.works[0].index[0].action, FileAction::CreateWork);

        // Analysis must not have written anything.
        assert_eq!(work_count(&lib).unwrap(), 0);

        let report = execute(&lib, &preview).unwrap();
        assert_eq!(report.works_created.len(), 1);
        assert_eq!(report.files_stored, 1);
        assert_eq!(work_count(&lib).unwrap(), 1);

        let stored = load_work(&lib, report.works_created[0]).unwrap().unwrap();
        assert_eq!(stored.title, "Alpha");
        assert_eq!(stored.author.as_deref(), Some("Someone"));
        assert_eq!(stored.language.as_deref(), Some("en"));
        assert_eq!(stored.word_count, 1234);
        assert_eq!(
            stored.word_count_source.as_deref(),
            Some("ao3"),
            "AO3 count is exact"
        );
        assert_eq!(stored.published_at.as_deref(), Some("2020-03-04"));
        assert_eq!(stored.published_prec.as_deref(), Some("day"));
        assert_eq!(report.stored_paths, vec!["000001/work.html".to_string()]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn original_and_translation_pair_automatically_in_one_import() {
        let root = temp_root("pair");
        let lib = Library::open_in_memory(&root).unwrap();

        let original = write(
            &root,
            "original.html",
            &ao3_html(
                "7929115",
                "One, two, three",
                "English",
                67851,
                r#"<ul><li>Translation into <span lang="zh">中文</span> available:
                   <a href="https://archiveofourown.org/works/14885858">一，二，三</a> by
                   <a rel="author" href="/users/l">lisabart</a></li></ul>"#,
            ),
        );
        let translation = write(
            &root,
            "translation.html",
            &ao3_html(
                "14885858",
                "一，二，三",
                "中文-普通话 國語",
                94736,
                r#"<ul><li>A translation of
                   <a href="https://archiveofourown.org/works/7929115">One, two, three</a> by
                   <a rel="author" href="/users/s">Severus_divides_into_H</a></li></ul>"#,
            ),
        );

        let preview = analyze(&lib, &[original, translation]).unwrap();
        assert_eq!(
            preview.works.len(),
            2,
            "two language editions are two works"
        );
        assert_eq!(preview.relations.len(), 1, "one pair: {preview:?}");

        let report = execute(&lib, &preview).unwrap();
        assert_eq!(report.works_created.len(), 2);
        assert_eq!(report.relations_stored, 1, "the pair is recorded once");

        let ids = &report.works_created;
        let stored = load_work(&lib, ids[0]).unwrap().unwrap();
        let other = load_work(&lib, ids[1]).unwrap().unwrap();
        assert_eq!(stored.translation_role.as_deref(), Some("original"));
        assert_eq!(other.translation_role.as_deref(), Some("translation"));

        // The relation is visible from both sides, which is what the symmetric table buys.
        assert_eq!(list_relations(&lib, ids[0]).unwrap().len(), 1);
        assert_eq!(list_relations(&lib, ids[1]).unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn importing_the_original_then_the_translation_completes_the_pair_later() {
        let root = temp_root("sequential");
        let lib = Library::open_in_memory(&root).unwrap();

        let original = write(
            &root,
            "original.html",
            &ao3_html(
                "7929115",
                "One, two, three",
                "English",
                67851,
                r#"<ul><li>Translation into <span lang="zh">中文</span> available:
                   <a href="https://archiveofourown.org/works/14885858">一，二，三</a></li></ul>"#,
            ),
        );

        // First import: the translation is only a name, so it must be remembered.
        let preview = analyze(&lib, &[original]).unwrap();
        let report = execute(&lib, &preview).unwrap();
        let original_id = report.works_created[0];
        assert_eq!(report.relations_stored, 0);
        assert_eq!(list_relations(&lib, original_id).unwrap().len(), 0);

        let pending = list_pending(&lib).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].external_id, "14885858");
        assert_eq!(pending[0].language.as_deref(), Some("zh"));

        // Second import: the translation arrives and the pair completes by itself.
        let translation = write(
            &root,
            "translation.html",
            &ao3_html(
                "14885858",
                "一，二，三",
                "中文-普通话 國語",
                94736,
                r#"<ul><li>A translation of
                   <a href="https://archiveofourown.org/works/7929115">One, two, three</a></li></ul>"#,
            ),
        );
        let preview = analyze(&lib, &[translation]).unwrap();
        let report = execute(&lib, &preview).unwrap();

        assert_eq!(report.relations_stored, 1, "the waiting pair is completed");
        assert_eq!(
            list_relations(&lib, original_id).unwrap().len(),
            1,
            "visible from the original's side too"
        );
        assert!(
            list_pending(&lib).unwrap().is_empty(),
            "the placeholder must be cleared once resolved"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn several_formats_of_one_work_group_into_a_single_work() {
        let root = temp_root("formats");
        let lib = Library::open_in_memory(&root).unwrap();

        // The same story saved three ways, all sharing the filename stem "a".
        let html = write(
            &root,
            "a.html",
            &ao3_html("555", "Alpha", "English", 5000, ""),
        );
        let txt = write(&root, "a.txt", "Some plain text body");
        let epub_path = root.join("a.epub");
        let mut epub_bytes = b"PK\x03\x04".to_vec();
        epub_bytes.extend_from_slice(&[0u8; 20]);
        epub_bytes.extend_from_slice(b"mimetypeapplication/epub+zip");
        epub_bytes.extend_from_slice(&[0u8; 40]);
        std::fs::write(&epub_path, &epub_bytes).unwrap();

        let preview = analyze(&lib, &[html, txt, epub_path]).unwrap();

        // The HTML declares an AO3 work id, so its key is the id; the TXT and EPUB have no
        // metadata and group by filename stem. Two groups, three files, one work each.
        assert_eq!(
            preview.works.len(),
            2,
            "expected ao3 group plus bare group: {preview:?}"
        );
        assert_eq!(preview.summary.files_total, 3);

        let ao3_work = preview
            .works
            .iter()
            .find(|w| w.external_id.as_deref() == Some("555"))
            .expect("the AO3 work");
        assert_eq!(ao3_work.index.len(), 1, "only the html carries the AO3 id");

        let bare = preview
            .works
            .iter()
            .find(|w| w.external_id.is_none())
            .expect("the metadata-free formats");
        assert_eq!(bare.index.len(), 2, "epub and txt share the stem 'a'");
        assert_eq!(bare.title, "a");
        let details: Vec<String> = bare.index.iter().map(|f| f.format.detail()).collect();
        assert!(details.contains(&"epub".to_string()));
        assert!(details.contains(&"txt".to_string()));

        let report = execute(&lib, &preview).unwrap();
        assert_eq!(report.files_stored, 3);
        assert_eq!(work_count(&lib).unwrap(), 2);
        // Both formats of the bare work land in one directory, one slot per format.
        let bare_id = report
            .works_created
            .iter()
            .copied()
            .find(|id| load_work(&lib, *id).unwrap().unwrap().title == "a")
            .expect("bare work id");
        assert_eq!(lib.files_for_work(bare_id).unwrap().len(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reimporting_the_same_file_is_recognised_and_skipped() {
        let root = temp_root("dupe");
        let lib = Library::open_in_memory(&root).unwrap();
        let src = write(
            &root,
            "story.html",
            &ao3_html("900", "Alpha", "English", 100, ""),
        );

        let preview = analyze(&lib, std::slice::from_ref(&src)).unwrap();
        execute(&lib, &preview).unwrap();
        assert_eq!(work_count(&lib).unwrap(), 1);

        // Same bytes, different filename: content hashing still catches it.
        let renamed = root.join("renamed.html");
        std::fs::copy(&src, &renamed).unwrap();

        let preview = analyze(&lib, &[renamed]).unwrap();
        assert_eq!(preview.summary.duplicates, 1);
        assert_eq!(preview.summary.files_to_store, 0);
        assert!(matches!(
            preview.works[0].index[0].action,
            FileAction::SkippedDuplicate { .. }
        ));

        let report = execute(&lib, &preview).unwrap();
        assert_eq!(report.files_stored, 0);
        assert_eq!(report.files_skipped, 1);
        assert_eq!(work_count(&lib).unwrap(), 1, "no second work created");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A metadata-free file whose *filename* does not correspond to any known work must not
    /// be attached to one by guesswork.
    ///
    /// This is the safer failure mode: a silent wrong merge corrupts an existing record,
    /// whereas an extra work that the user can merge by hand costs one click. (The "link
    /// to…" action from the plan covers exactly this case.)
    #[test]
    fn a_bare_file_with_an_unrelated_name_does_not_merge_into_an_ao3_work() {
        let root = temp_root("noguess");
        let lib = Library::open_in_memory(&root).unwrap();

        // The AO3 file is titled "Alpha" and lives in story.html.
        let html = write(
            &root,
            "story.html",
            &ao3_html("777", "Alpha", "English", 5000, ""),
        );
        let preview = analyze(&lib, &[html]).unwrap();
        let report = execute(&lib, &preview).unwrap();
        let work_id = report.works_created[0];

        // A bare epub named story.epub carries no id and its stem ("story") is not the
        // work's title ("Alpha"), so there is nothing to justify joining them.
        let epub_path = root.join("story.epub");
        let mut bytes = b"PK\x03\x04".to_vec();
        bytes.extend_from_slice(&[0u8; 20]);
        bytes.extend_from_slice(b"mimetypeapplication/epub+zip");
        bytes.extend_from_slice(&[0u8; 40]);
        std::fs::write(&epub_path, &bytes).unwrap();

        let preview = analyze(&lib, &[epub_path]).unwrap();
        assert_eq!(preview.works.len(), 1);
        assert_eq!(
            preview.works[0].existing_work_id, None,
            "no id and a non-matching name means no basis to attach"
        );
        assert_eq!(preview.works[0].title, "story");
        assert_eq!(preview.works[0].index[0].action, FileAction::CreateWork);

        execute(&lib, &preview).unwrap();
        assert_eq!(
            work_count(&lib).unwrap(),
            2,
            "the AO3 work is left untouched"
        );
        assert_eq!(
            load_work(&lib, work_id).unwrap().unwrap().title,
            "Alpha",
            "the existing work's metadata must not be altered"
        );
        assert_eq!(
            lib.files_for_work(work_id).unwrap().len(),
            1,
            "still just its html"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_bare_txt_named_after_the_work_attaches_to_it() {
        let root = temp_root("titlematch");
        let lib = Library::open_in_memory(&root).unwrap();

        let html = write(
            &root,
            "Alpha.html",
            &ao3_html("777", "Alpha", "English", 5000, ""),
        );
        let preview = analyze(&lib, &[html]).unwrap();
        let report = execute(&lib, &preview).unwrap();
        let work_id = report.works_created[0];

        // Same title, no AO3 id: the filename stem matches the work's title, which is
        // enough evidence to attach rather than create a near-duplicate entry.
        let txt = write(&root, "Alpha.txt", "plain text of the same story");
        let preview = analyze(&lib, &[txt]).unwrap();

        assert_eq!(preview.works.len(), 1);
        assert_eq!(preview.works[0].existing_work_id, Some(work_id));
        assert_eq!(
            preview.works[0].index[0].action,
            FileAction::AttachToExisting
        );

        let report = execute(&lib, &preview).unwrap();
        assert_eq!(report.works_created.len(), 0, "no duplicate work");
        assert_eq!(work_count(&lib).unwrap(), 1);
        assert_eq!(
            lib.files_for_work(work_id).unwrap().len(),
            2,
            "html plus txt"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn importing_another_format_attaches_to_the_existing_work() {
        let root = temp_root("attach");
        let lib = Library::open_in_memory(&root).unwrap();

        let html = write(
            &root,
            "story.html",
            &ao3_html("777", "Alpha", "English", 5000, ""),
        );
        let preview = analyze(&lib, &[html]).unwrap();
        let report = execute(&lib, &preview).unwrap();
        let work_id = report.works_created[0];

        // A second file of the same AO3 work, exported as TXT. AO3's TXT export is derived
        // from the same work, but the file itself carries no id — so the filename is what
        // ties it back, and here it matches the work's title.
        let txt = write(&root, "Alpha.txt", "same story, plain text");
        let preview = analyze(&lib, &[txt]).unwrap();
        assert_eq!(preview.works.len(), 1);
        assert_eq!(preview.works[0].existing_work_id, Some(work_id));
        assert_eq!(
            preview.works[0].index[0].action,
            FileAction::AttachToExisting
        );

        let report = execute(&lib, &preview).unwrap();
        assert_eq!(report.works_created.len(), 0, "no new work");
        assert_eq!(report.works_matched, vec![work_id]);
        assert_eq!(work_count(&lib).unwrap(), 1);
        assert_eq!(lib.files_for_work(work_id).unwrap().len(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reimporting_the_same_format_replaces_the_stored_file() {
        let root = temp_root("replaceformat");
        let lib = Library::open_in_memory(&root).unwrap();

        let html = write(
            &root,
            "story.html",
            &ao3_html("888", "Alpha", "English", 10, ""),
        );
        let preview = analyze(&lib, &[html]).unwrap();
        let report = execute(&lib, &preview).unwrap();
        let work_id = report.works_created[0];

        // New content, same work id and format, with replacement explicitly requested.
        let updated = write(
            &root,
            "story-v2.html",
            &ao3_html("888", "Alpha Revised", "English", 20, ""),
        );
        let preview = analyze_with(
            &lib,
            &[updated],
            ImportOptions {
                version_preference: VersionPreference::Replace,
            },
            &[],
        )
        .unwrap();
        assert!(matches!(
            preview.works[0].index[0].action,
            FileAction::ReplacesStoredFile { .. }
        ));

        execute(&lib, &preview).unwrap();
        assert_eq!(
            lib.files_for_work(work_id).unwrap().len(),
            1,
            "one slot per format when replacing"
        );
        // The title is not overwritten by a re-import, so a manual edit survives.
        assert_eq!(load_work(&lib, work_id).unwrap().unwrap().title, "Alpha");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The behaviour versions exist for: a re-imported revision is kept alongside the original
    /// rather than overwriting it.
    #[test]
    fn reimporting_keeps_the_earlier_revision_by_default() {
        let root = temp_root("keepboth");
        let lib = Library::open_in_memory(&root).unwrap();

        let first = write(
            &root,
            "story.html",
            &ao3_html("888", "Alpha", "English", 10, ""),
        );
        let preview = analyze(&lib, &[first]).unwrap();
        let report = execute(&lib, &preview).unwrap();
        let work_id = report.works_created[0];

        // The work was updated or reposted: same AO3 id, different content.
        let revised = write(
            &root,
            "story-revised.html",
            &ao3_html("888", "Alpha", "English", 25, ""),
        );
        let preview = analyze(&lib, &[revised]).unwrap();
        assert_eq!(
            preview.works[0].index[0].action,
            FileAction::KeepBothVersions {
                existing_work_id: work_id
            },
            "the default is to keep both"
        );

        let report = execute(&lib, &preview).unwrap();
        assert_eq!(report.files_stored, 1);

        let files = lib.files_for_work(work_id).unwrap();
        assert_eq!(files.len(), 2, "both revisions are in the library");

        // Exactly one current version, and it is the newer one.
        let current: Vec<&crate::store::FileRecord> =
            files.iter().filter(|f| f.is_current()).collect();
        assert_eq!(current.len(), 1, "exactly one current version per format");
        assert_eq!(current[0].version, 2);
        assert_eq!(current[0].original_name, "story-revised.html");

        let archived: Vec<&crate::store::FileRecord> =
            files.iter().filter(|f| !f.is_current()).collect();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].version, 1);
        assert_eq!(archived[0].original_name, "story.html");

        // Both files are physically present, in versioned paths.
        for file in &files {
            assert!(
                lib.abs_path(&file.rel_path).is_file(),
                "missing on disk: {}",
                file.rel_path
            );
        }
        assert!(files.iter().any(|f| f.rel_path.ends_with("work.html")));
        assert!(files.iter().any(|f| f.rel_path.ends_with("work.v2.html")));

        // Promoting the older revision swaps which one is current.
        lib.set_current_version(archived[0].id).unwrap();
        let after = lib.files_for_work(work_id).unwrap();
        let now_current: Vec<&crate::store::FileRecord> =
            after.iter().filter(|f| f.is_current()).collect();
        assert_eq!(now_current.len(), 1, "still exactly one current");
        assert_eq!(now_current[0].version, 1);

        // The current version cannot be deleted out from under the work.
        assert!(lib.delete_version(now_current[0].id).is_err());
        // An archived one can.
        let still_archived = after.iter().find(|f| !f.is_current()).unwrap();
        lib.delete_version(still_archived.id).unwrap();
        assert_eq!(lib.files_for_work(work_id).unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Importing the identical file again must not create a second version.
    #[test]
    fn reimporting_identical_content_does_not_create_a_version() {
        let root = temp_root("nodupversion");
        let lib = Library::open_in_memory(&root).unwrap();
        let src = write(
            &root,
            "story.html",
            &ao3_html("999", "Alpha", "English", 10, ""),
        );

        let preview = analyze(&lib, std::slice::from_ref(&src)).unwrap();
        let report = execute(&lib, &preview).unwrap();
        let work_id = report.works_created[0];

        let preview = analyze(&lib, &[src]).unwrap();
        assert_eq!(preview.summary.duplicates, 1);
        execute(&lib, &preview).unwrap();
        assert_eq!(
            lib.files_for_work(work_id).unwrap().len(),
            1,
            "identical content is not a new version"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The case the automatic rules cannot solve, and the reason a manual route exists.
    ///
    /// An AO3 download is stored under its *title*, while the file is named however the site
    /// named it. `Zong_Lu_Jian_Dan_Di_Sha.epub` has no textual overlap with
    /// `【粽驴】简单地杀个人`, so nothing justifies merging them automatically — but the user can
    /// say so, and then the files must genuinely join that work.
    #[test]
    fn a_manual_choice_merges_files_the_rules_would_keep_apart() {
        let root = temp_root("manualmerge");
        let lib = Library::open_in_memory(&root).unwrap();

        // The work as a real AO3 download names it: a Chinese title.
        let html = write(
            &root,
            "Zong_Lu_Jian_Dan_Di_Sha.html",
            &ao3_html(
                "91520771",
                "【粽驴】简单地杀个人",
                "中文-普通话 國語",
                22210,
                "",
            ),
        );
        let preview = analyze(&lib, &[html]).unwrap();
        let report = execute(&lib, &preview).unwrap();
        let work_id = report.works_created[0];

        // A bare EPUB of the same work, named differently. Nothing links the two.
        let epub_path = root.join("Zong_Lu_Jian_Dan_Di_Sha.epub");
        let mut bytes = b"PK\x03\x04".to_vec();
        bytes.extend_from_slice(&[0u8; 20]);
        bytes.extend_from_slice(b"mimetypeapplication/epub+zip");
        bytes.extend_from_slice(&[0u8; 40]);
        std::fs::write(&epub_path, &bytes).unwrap();

        let automatic = analyze(&lib, std::slice::from_ref(&epub_path)).unwrap();
        assert_eq!(automatic.works.len(), 1);
        assert_eq!(
            automatic.works[0].existing_work_id, None,
            "no evidence means no automatic merge"
        );
        assert!(automatic.works[0].manual_target.is_none());

        // Now the user says which work it belongs to.
        let key = automatic.works[0].key.clone();
        let assigned = analyze_with(
            &lib,
            std::slice::from_ref(&epub_path),
            ImportOptions::default(),
            &[ManualAssignment {
                key: key.clone(),
                work_id,
            }],
        )
        .unwrap();

        assert_eq!(assigned.works.len(), 1);
        assert_eq!(assigned.works[0].existing_work_id, Some(work_id));
        assert_eq!(
            assigned.works[0].manual_target,
            Some(work_id),
            "the decision is recorded as the user's, not as an automatic match"
        );
        assert_eq!(assigned.summary.works_new, 0, "no new work");
        assert_eq!(assigned.summary.works_existing, 1);

        let report = execute(&lib, &assigned).unwrap();
        assert!(report.works_created.is_empty(), "nothing new was created");
        assert_eq!(report.works_matched, vec![work_id]);
        assert_eq!(report.files_stored, 1);

        // Both formats now hang off the one work.
        let files = lib.files_for_work(work_id).unwrap();
        assert_eq!(files.len(), 2);
        let formats: Vec<&str> = files.iter().map(|f| f.format_detail.as_str()).collect();
        assert!(formats.contains(&"html"));
        assert!(formats.contains(&"epub"));

        // And the work's own metadata was not overwritten by the merge.
        let stored = load_work(&lib, work_id).unwrap().unwrap();
        assert_eq!(stored.title, "【粽驴】简单地杀个人");
        assert_eq!(stored.word_count, 22_210, "AO3's count survives");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Assigning to a work that already holds the format must ask for a version decision.
    #[test]
    fn a_manual_merge_into_a_work_holding_that_format_needs_a_version_choice() {
        let root = temp_root("manualversion");
        let lib = Library::open_in_memory(&root).unwrap();

        let first = write(
            &root,
            "Alpha.html",
            &ao3_html("555", "Alpha", "English", 10, ""),
        );
        let preview = analyze(&lib, &[first]).unwrap();
        let report = execute(&lib, &preview).unwrap();
        let work_id = report.works_created[0];

        // A different file of the same format, with no id to link it.
        let revised = write(&root, "Alpha-revised.html", "<html>revised</html>");
        let automatic = analyze(&lib, std::slice::from_ref(&revised)).unwrap();
        let key = automatic.works[0].key.clone();
        // Without the manual choice this would be a work of its own, so no version question.
        assert!(!automatic.works[0].index[0].action.needs_version_choice());

        let assigned = analyze_with(
            &lib,
            std::slice::from_ref(&revised),
            ImportOptions::default(),
            &[ManualAssignment { key, work_id }],
        )
        .unwrap();
        assert!(
            assigned.works[0].index[0].action.needs_version_choice(),
            "merging into a work that holds this format is a version decision: {:?}",
            assigned.works[0].index[0].action
        );
        assert!(matches!(
            assigned.works[0].index[0].action,
            FileAction::KeepBothVersions { existing_work_id } if existing_work_id == work_id
        ));

        execute(&lib, &assigned).unwrap();
        let files = lib.files_for_work(work_id).unwrap();
        assert_eq!(
            files.len(),
            2,
            "the revision joined the original as a version"
        );
        assert_eq!(files.iter().filter(|f| f.is_current()).count(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// An assignment can be undone, and an unknown key is simply ignored.
    #[test]
    fn a_manual_choice_can_be_withdrawn() {
        let root = temp_root("withdraw");
        let lib = Library::open_in_memory(&root).unwrap();
        let existing = lib
            .insert_work(&NewWork {
                title: "Target".into(),
                ..Default::default()
            })
            .unwrap();

        let src = write(&root, "loose.txt", "some text");
        let preview = analyze(&lib, std::slice::from_ref(&src)).unwrap();
        let key = preview.works[0].key.clone();

        let assigned = analyze_with(
            &lib,
            std::slice::from_ref(&src),
            ImportOptions::default(),
            &[ManualAssignment {
                key: key.clone(),
                work_id: existing,
            }],
        )
        .unwrap();
        assert_eq!(assigned.works[0].existing_work_id, Some(existing));

        // Withdrawing it returns the plan to the automatic result.
        let withdrawn = analyze_with(&lib, &[src], ImportOptions::default(), &[]).unwrap();
        assert_eq!(withdrawn.works[0].existing_work_id, None);
        assert_eq!(withdrawn.summary.works_new, 1);

        // An assignment for a key that is not in the batch has no effect on other works.
        let stale = analyze_with(
            &lib,
            &[write(&root, "other.txt", "different")],
            ImportOptions::default(),
            &[ManualAssignment {
                key: "stem:does-not-exist".into(),
                work_id: existing,
            }],
        )
        .unwrap();
        assert_eq!(
            stale.works[0].existing_work_id, None,
            "unrelated key is ignored"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_txt_without_ao3_metadata_becomes_its_own_work_with_an_estimated_count() {
        let root = temp_root("txt");
        let lib = Library::open_in_memory(&root).unwrap();
        let src = write(&root, "Some Story.txt", "你好世界 hello world");

        let preview = analyze(&lib, &[src]).unwrap();
        assert_eq!(preview.works.len(), 1);
        assert_eq!(
            preview.works[0].title, "Some Story",
            "filename is the only title"
        );
        assert_eq!(preview.works[0].external_id, None);
        assert!(preview.works[0].index[0].word_count_is_estimated);

        let report = execute(&lib, &preview).unwrap();
        let stored = load_work(&lib, report.works_created[0]).unwrap().unwrap();
        assert_eq!(stored.word_count, 6, "4 Han characters + 2 latin words");
        assert_eq!(stored.word_count_source.as_deref(), Some("estimated"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn two_unrelated_bare_txt_files_do_not_merge() {
        let root = temp_root("twotxt");
        let lib = Library::open_in_memory(&root).unwrap();
        let a = write(&root, "First Story.txt", "one");
        let b = write(&root, "Second Story.txt", "two");

        let preview = analyze(&lib, &[a, b]).unwrap();
        assert_eq!(
            preview.works.len(),
            2,
            "no metadata means no basis to merge: {preview:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_is_imported_recursively() {
        let root = temp_root("recursive");
        let lib = Library::open_in_memory(&root).unwrap();
        let nested = root.join("downloads").join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        write(
            &nested,
            "a.html",
            &ao3_html("1", "Alpha", "English", 10, ""),
        );
        write(&root.join("downloads"), "b.txt", "some text");
        // Hidden files must be ignored.
        write(&nested, ".DS_Store", "junk");
        write(&nested, "._resource.txt", "junk");

        let files = collect_files(&[root.join("downloads")]).unwrap();
        assert_eq!(
            files.len(),
            2,
            "hidden and resource-fork files excluded: {files:?}"
        );

        let preview = analyze(&lib, &[root.join("downloads")]).unwrap();
        assert_eq!(preview.works.len(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pdf_contributes_a_file_record_without_a_text_estimate() {
        let root = temp_root("pdf");
        let lib = Library::open_in_memory(&root).unwrap();
        let src = root.join("scan.pdf");
        // PDF text extraction is not implemented, so no word count may be invented.
        std::fs::write(&src, b"%PDF-1.4\n%binary-ish content").unwrap();

        let preview = analyze(&lib, &[src]).unwrap();
        assert_eq!(preview.works[0].index[0].format.detail(), "pdf");
        assert_eq!(preview.works[0].index[0].word_count, None);
        assert_eq!(preview.works[0].index[0].format.class().as_str(), "pdf");

        let report = execute(&lib, &preview).unwrap();
        let files = lib.files_for_work(report.works_created[0]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].format_class.as_str(), "pdf");
        assert_eq!(files[0].word_count, None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn analysis_reports_unreadable_files_instead_of_importing_nothing_silently() {
        let root = temp_root("missing");
        let lib = Library::open_in_memory(&root).unwrap();
        let missing = root.join("nope.html");
        let err = analyze(&lib, &[missing]).unwrap_err();
        assert!(err.to_string().contains("nope.html"), "got: {err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn summary_counts_works_and_files_separately() {
        let root = temp_root("summary");
        let lib = Library::open_in_memory(&root).unwrap();
        let html = write(&root, "a.html", &ao3_html("11", "Alpha", "English", 10, ""));
        let txt = write(&root, "b.txt", "words");
        let preview = analyze(&lib, &[html, txt]).unwrap();
        assert_eq!(preview.summary.files_total, 2);
        assert_eq!(preview.summary.files_to_store, 2);
        assert_eq!(preview.summary.duplicates, 0);
        assert_eq!(preview.summary.works_new, 2);

        // And after importing, the same file is a duplicate rather than a new work.
        execute(&lib, &preview).unwrap();
        let again = analyze(
            &lib,
            &[write(
                &root,
                "a.html",
                &ao3_html("11", "Alpha", "English", 10, ""),
            )],
        )
        .unwrap();
        assert_eq!(again.summary.duplicates, 1);
        assert_eq!(work_count(&lib).unwrap(), 2, "still two works, not three");
        let _ = std::fs::remove_dir_all(&root);
    }
}
