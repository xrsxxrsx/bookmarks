//! Desktop shell for the local fanfiction library.
//!
//! This crate owns only what is specific to the running application: the window, the command
//! surface exposed to the frontend, and the application's settings. All domain logic —
//! parsing, normalisation, storage, import — lives in `bookmarks-core` so it can be tested
//! without a window.
//!
//! The library's location is a **setting, not a constant**, which shapes the state machine
//! here: `%APPDATA%` keeps only a small `settings.json`, and the database plus every imported
//! file live wherever the user points it. So there are states before a library exists, and the
//! interface has to render "no library yet" and "that library would not open" as well as a
//! loaded library.
//!
//! The command surface is deliberately thin. The whole library is read in one call and the
//! frontend does its own filtering, sorting, grouping and searching, so those behaviours live
//! in one place rather than being split between SQL and TypeScript.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

use bookmarks_core::config::{self, GroupMode, Look, Settings, Theme};
use bookmarks_core::import::{self, FileAction, ImportOptions, ImportReport, VersionPreference};
use bookmarks_core::store::{Library, SeriesMembership};
use bookmarks_core::view::{self, LibraryView};

/// Application identifier. It fixes the `%APPDATA%` directory holding `settings.json`, so it
/// must never change: a different identifier means a different settings file, and the user's
/// chosen library location would appear to have been forgotten.
pub const IDENTIFIER: &str = "com.bookmarks.app";

/// Where a library is created when the user has not chosen a place.
///
/// Resolved from the user's profile rather than hard-coded, so a fresh clone does not create a
/// library on whatever drive the original author happened to use. `Documents` is the right place
/// semantically — these are the user's own files — and it is somewhere they can find and back up.
///
/// Outside the application data directory deliberately: the library is the large part, so
/// `%APPDATA%` keeps only the few hundred bytes of settings. The trade-off is that `Documents`
/// usually sits on C:, while the whole point of a chosen location is that a nearly full system
/// drive cannot stop the library working — hence the setting.
fn default_library_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join("Documents").join("bookmarks")
}

/// The user's data directory (`%APPDATA%` on Windows), where settings live.
pub fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(IDENTIFIER)
}

/// The library location used when none has been chosen.
///
/// `BOOKMARKS_LIBRARY` overrides it, which keeps tests and scratch runs off the real default.
pub fn default_library_path() -> PathBuf {
    std::env::var_os("BOOKMARKS_LIBRARY")
        .map(PathBuf::from)
        .unwrap_or_else(default_library_dir)
}

/// Opens the library at an explicit path, creating it if needed.
pub fn open_library_at(path: &Path) -> bookmarks_core::Result<Library> {
    Library::open(path)
}

/// Reads a database's schema version without migrating it, so a newer library can be detected
/// and refused rather than silently downgraded.
pub fn inspect_database_version(path: &Path) -> bookmarks_core::Result<i64> {
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    read_version(&conn)
}

fn read_version(conn: &rusqlite::Connection) -> bookmarks_core::Result<i64> {
    let has_table: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_migrations'",
        [],
        |row| row.get(0),
    )?;
    if has_table == 0 {
        return Ok(0);
    }
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?)
}

// ---------------------------------------------------------------- errors

/// Errors cross the IPC boundary as text, so the message must read well on its own rather than
/// being a debug dump.
#[derive(Debug, Serialize)]
pub struct CmdError {
    message: String,
}

impl CmdError {
    fn new(message: impl Into<String>) -> Self {
        CmdError {
            message: message.into(),
        }
    }
}

impl From<bookmarks_core::Error> for CmdError {
    fn from(e: bookmarks_core::Error) -> Self {
        CmdError::new(e.to_string())
    }
}

impl From<std::io::Error> for CmdError {
    fn from(e: std::io::Error) -> Self {
        CmdError::new(e.to_string())
    }
}

type CmdResult<T> = Result<T, CmdError>;

// ---------------------------------------------------------------- command state

/// A preview the user has seen but not yet confirmed.
///
/// Only the source paths and the decisions are kept: the plan is rebuilt from them whenever
/// something changes. Holding the parsed plan here would mean trusting it after the library
/// changed underneath, and re-reading a handful of files is cheap.
struct PendingImport {
    paths: Vec<PathBuf>,
    options: ImportOptions,
    /// Reassignments the user made by hand, keyed by preview work key.
    manual: Vec<import::ManualAssignment>,
}

/// Everything a command needs.
///
/// The library is optional because the application can be running with no library configured,
/// or with one that failed to open. Both are states the interface renders rather than crashes
/// on, which is what makes the library's location a setting rather than a constant.
///
/// It sits behind a `Mutex` because `rusqlite::Connection` is `Send` but not `Sync`, while
/// Tauri's managed state must be both. One lock covers the whole application, which is the
/// right trade at this scale: commands are short and run one at a time anyway.
pub struct AppState {
    inner: Mutex<Inner>,
    /// Monotonic source of preview tokens.
    next_token: Mutex<u64>,
}

struct Inner {
    settings: Settings,
    library: Option<Library>,
    /// Why the library is not open, when it is not.
    library_error: Option<String>,
    /// Previews awaiting confirmation, keyed by the token handed to the frontend.
    pending: HashMap<String, PendingImport>,
}

impl AppState {
    /// Reads settings, resolves the library location and tries to open it.
    ///
    /// Never fails: a library that cannot be opened becomes `library_error` and the window
    /// reports it, because refusing to start would leave the user no way to fix the path.
    fn new(data_dir: PathBuf) -> Self {
        let mut settings = config::load(&data_dir);
        let path = settings
            .library_path
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(default_library_path);

        let (library, error) = match Library::open(&path) {
            Ok(lib) => (Some(lib), None),
            Err(e) => (None, Some(format!("{}：{e}", path.display()))),
        };

        // Record the resolved path so the settings file always states where the library is,
        // even when the user never chose explicitly.
        settings.library_path = Some(path.display().to_string());
        let _ = config::save(&data_dir, &settings);

        AppState {
            inner: Mutex::new(Inner {
                settings,
                library,
                library_error: error,
                pending: HashMap::new(),
            }),
            next_token: Mutex::new(1),
        }
    }
}

/// Locks the application state.
fn state_lock<'a>(state: &'a State<'_, AppState>) -> CmdResult<std::sync::MutexGuard<'a, Inner>> {
    state
        .inner
        .lock()
        .map_err(|_| CmdError::new("internal state was poisoned by an earlier failure"))
}

/// Runs `f` with the open library, or reports why there is none.
fn with_library<T>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&Library) -> CmdResult<T>,
) -> CmdResult<T> {
    let inner = state_lock(state)?;
    match inner.library.as_ref() {
        Some(lib) => f(lib),
        None => Err(CmdError::new(
            inner
                .library_error
                .clone()
                .unwrap_or_else(|| "还没有打开书库".to_owned()),
        )),
    }
}

// ---------------------------------------------------------------- read models

/// The counts shown in the status bar. Works and files are counted separately because "how many
/// fics do I have" is a different question from "how many files are on disk".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountsDto {
    pub works_total: i64,
    pub works_with_file: i64,
    pub works_record_only: i64,
    pub files_total: i64,
    pub works_related: i64,
    pub pending_relations: i64,
}

impl From<view::LibraryCounts> for CountsDto {
    fn from(c: view::LibraryCounts) -> Self {
        CountsDto {
            works_total: c.works_total,
            works_with_file: c.works_with_file,
            works_record_only: c.works_record_only,
            files_total: c.files_total,
            works_related: c.works_related,
            pending_relations: c.pending_relations,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDto {
    pub id: i64,
    pub format_class: String,
    pub format_detail: String,
    pub original_name: String,
    pub rel_path: String,
    pub size_bytes: i64,
    pub word_count: Option<i64>,
    pub is_estimated: bool,
    /// Whether the file is present on disk. It can go missing if deleted outside the
    /// application, and the interface should say so rather than fail only when opened.
    pub exists: bool,
    /// `current` or `archived`.
    pub role: String,
    pub version: i64,
}

impl From<view::FileView> for FileDto {
    fn from(f: view::FileView) -> Self {
        FileDto {
            id: f.id,
            format_class: f.format_class,
            format_detail: f.format_detail,
            original_name: f.original_name,
            rel_path: f.rel_path,
            size_bytes: f.size_bytes,
            word_count: f.word_count,
            is_estimated: f.is_estimated,
            exists: f.exists,
            role: f.role,
            version: f.version,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedDto {
    pub work_id: i64,
    pub title: String,
    pub author: Option<String>,
    pub language: Option<String>,
    pub kind: String,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesDto {
    pub series_id: i64,
    pub name: String,
    pub position: Option<i64>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkDto {
    pub id: i64,
    pub title: String,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub my_comment: Option<String>,
    pub published_at: Option<String>,
    pub published_prec: Option<String>,
    pub date_is_approx: bool,
    pub published_display: Option<String>,
    pub completed_at: Option<String>,
    pub completed_prec: Option<String>,
    pub completed_is_approx: bool,
    pub completed_display: Option<String>,
    pub word_count: i64,
    pub word_count_source: Option<String>,
    pub word_count_display: String,
    pub language: Option<String>,
    pub language_label: Option<String>,
    pub translation_role: Option<String>,
    pub source_url: Option<String>,
    pub external_id: Option<String>,
    pub ao3_tags: Vec<String>,
    pub tags: Vec<String>,
    pub status: String,
    pub files: Vec<FileDto>,
    pub related: Vec<RelatedDto>,
    pub series: Vec<SeriesDto>,
    /// Already folded for matching, so the frontend does not reimplement the rules.
    pub search_text: String,
}

impl From<view::WorkView> for WorkDto {
    fn from(w: view::WorkView) -> Self {
        WorkDto {
            files: w.files.into_iter().map(FileDto::from).collect(),
            related: w
                .related
                .into_iter()
                .map(|r| RelatedDto {
                    work_id: r.work_id,
                    title: r.title,
                    author: r.author,
                    language: r.language,
                    kind: r.kind,
                    origin: r.origin,
                })
                .collect(),
            series: w
                .series
                .into_iter()
                .map(|s| SeriesDto {
                    series_id: s.series_id,
                    name: s.name,
                    position: s.position,
                    total: s.total,
                })
                .collect(),
            id: w.id,
            title: w.title,
            author: w.author,
            summary: w.summary,
            my_comment: w.my_comment,
            published_at: w.published_at,
            published_prec: w.published_prec,
            date_is_approx: w.date_is_approx,
            published_display: w.published_display,
            completed_at: w.completed_at,
            completed_prec: w.completed_prec,
            completed_is_approx: w.completed_is_approx,
            completed_display: w.completed_display,
            word_count: w.word_count,
            word_count_source: w.word_count_source,
            word_count_display: w.word_count_display,
            language: w.language,
            language_label: w.language_label,
            translation_role: w.translation_role,
            source_url: w.source_url,
            external_id: w.external_id,
            ao3_tags: w.ao3_tags,
            tags: w.tags,
            status: w.status,
            search_text: w.search_text,
        }
    }
}

/// A relation to a work that has not been imported, shown as a suggestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingDto {
    pub external_id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub from_title: String,
}

impl From<view::PendingView> for PendingDto {
    fn from(p: view::PendingView) -> Self {
        PendingDto {
            external_id: p.external_id,
            title: p.title,
            author: p.author,
            language: p.language,
            from_title: p.from_title,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsDto {
    pub library_path: String,
    /// The location used when none has been chosen yet.
    pub default_library_path: String,
    pub settings_file: String,
    pub theme: String,
    pub font_size_px: u16,
    pub group_mode: String,
    pub keep_both_versions: bool,
    pub font_size_choices: Vec<u16>,
}

impl SettingsDto {
    fn build(settings: &Settings) -> Self {
        SettingsDto {
            library_path: settings.library_path.clone().unwrap_or_default(),
            default_library_path: default_library_path().display().to_string(),
            settings_file: config::settings_file(&data_dir()).display().to_string(),
            theme: settings.theme.as_str().to_owned(),
            font_size_px: settings.font_size_px,
            group_mode: settings.group_mode.as_str().to_owned(),
            keep_both_versions: settings.keep_both_versions,
            font_size_choices: config::FONT_SIZE_CHOICES.to_vec(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryDto {
    pub works: Vec<WorkDto>,
    pub pending: Vec<PendingDto>,
    pub all_tags: Vec<String>,
    pub counts: CountsDto,
    pub library_root: String,
    /// Every series name in use, for the editor's suggestions.
    pub all_series: Vec<String>,
}

impl LibraryDto {
    fn build(lib: &Library) -> CmdResult<Self> {
        let view: LibraryView = view::library_view(lib)?;
        Ok(LibraryDto {
            works: view.works.into_iter().map(WorkDto::from).collect(),
            pending: view.pending.into_iter().map(PendingDto::from).collect(),
            all_tags: view.all_tags,
            counts: view.counts.into(),
            library_root: lib.root().display().to_string(),
            all_series: view::all_series(lib)?,
        })
    }
}

/// What the window needs before it can render anything.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupDto {
    /// `open` when the library loaded, `unconfigured` when no location has been chosen, `error`
    /// when it could not be opened.
    pub state: String,
    pub library_path: String,
    pub default_library_path: String,
    pub error: Option<String>,
    pub settings: SettingsDto,
}

fn startup_dto(inner: &Inner) -> StartupDto {
    let path = inner
        .settings
        .library_path
        .clone()
        .unwrap_or_else(|| default_library_path().display().to_string());
    let state_name = if inner.library.is_some() {
        "open"
    } else if inner.settings.library_path.is_none() {
        "unconfigured"
    } else {
        "error"
    };
    StartupDto {
        state: state_name.to_owned(),
        library_path: path,
        default_library_path: default_library_path().display().to_string(),
        error: inner.library_error.clone(),
        settings: SettingsDto::build(&inner.settings),
    }
}

// ---------------------------------------------------------------- startup and settings

/// Reads settings and reports whether a library is open.
#[tauri::command]
fn startup(state: State<'_, AppState>) -> CmdResult<StartupDto> {
    let inner = state_lock(&state)?;
    Ok(startup_dto(&inner))
}

/// Reads the whole library.
#[tauri::command]
fn load_library(state: State<'_, AppState>) -> CmdResult<LibraryDto> {
    with_library(&state, LibraryDto::build)
}

/// Saves interface settings.
///
/// The library path is not changed here: moving a library is a separate, explicit operation
/// (`set_library_path`) because it can move files on disk.
#[tauri::command]
fn save_settings(
    state: State<'_, AppState>,
    theme: Option<String>,
    font_size_px: Option<u16>,
    group_mode: Option<String>,
    keep_both_versions: Option<bool>,
) -> CmdResult<SettingsDto> {
    let mut inner = state_lock(&state)?;
    if let Some(theme) = theme.as_deref().and_then(Theme::parse) {
        inner.settings.theme = theme;
    }
    if let Some(size) = font_size_px {
        if config::FONT_SIZE_CHOICES.contains(&size) {
            inner.settings.font_size_px = size;
        }
    }
    if let Some(mode) = group_mode.as_deref().and_then(GroupMode::parse) {
        inner.settings.group_mode = mode;
    }
    if let Some(keep) = keep_both_versions {
        inner.settings.keep_both_versions = keep;
    }
    let settings = inner.settings.clone();
    config::save(&data_dir(), &settings)?;
    Ok(SettingsDto::build(&settings))
}

/// Points the application at a library, optionally **moving** the current one there.
///
/// Refuses a destination that already holds something, because overwriting another library
/// would destroy data. When moving, the copy is verified before the original is removed, so a
/// failure leaves the library where it was.
#[tauri::command]
fn set_library_path(
    state: State<'_, AppState>,
    path: String,
    move_existing: bool,
) -> CmdResult<StartupDto> {
    let target = PathBuf::from(path.trim());
    if target.as_os_str().is_empty() {
        return Err(CmdError::new("请选择一个目录"));
    }

    if move_existing {
        return move_library_to(&state, &target);
    }

    // Not moving: the target must already be a library, or be new/empty.
    match config::inspect(&target) {
        Look::IsLibrary | Look::Empty => {}
        Look::OccupiedByOther => {
            return Err(CmdError::new(format!(
                "{} 既不是空的，也不像一个书库。请选一个空目录，或选中已有的书库目录。",
                target.display()
            )))
        }
    }

    let mut inner = state_lock(&state)?;
    let (library, error) = match Library::open(&target) {
        Ok(lib) => (Some(lib), None),
        Err(e) => (None, Some(format!("{}：{e}", target.display()))),
    };
    inner.library = library;
    inner.library_error = error;
    inner.settings.library_path = Some(target.display().to_string());
    let settings = inner.settings.clone();
    config::save(&data_dir(), &settings)?;
    Ok(startup_dto(&inner))
}

/// Moves the open library to `target`, verifying the copy before removing the original.
fn move_library_to(state: &State<'_, AppState>, target: &Path) -> CmdResult<StartupDto> {
    // Nothing may hold the old location open while it is moved, so the connection is dropped
    // first. On failure it is reopened, so the application is never left without a library.
    let (from, settings) = {
        let mut inner = state_lock(state)?;
        let Some(from) = inner.settings.library_path.clone().map(PathBuf::from) else {
            return Err(CmdError::new("还没有书库可以移动"));
        };
        if inner.library.is_none() {
            return Err(CmdError::new(
                "当前书库没有成功打开，无法自动移动。可以手动复制文件，或先修好当前书库。",
            ));
        }
        inner.library = None;
        let settings = inner.settings.clone();
        (from, settings)
    };

    if let Err(e) = config::move_library(&from, target, |_| {}) {
        let mut inner = state_lock(state)?;
        inner.library = Library::open(&from).ok();
        return Err(e.into());
    }

    let mut inner = state_lock(state)?;
    let (library, error) = match Library::open(target) {
        Ok(lib) => (Some(lib), None),
        Err(e) => (None, Some(format!("{}：{e}", target.display()))),
    };
    inner.library = library;
    inner.library_error = error;
    let mut settings = settings;
    settings.library_path = Some(target.display().to_string());
    inner.settings = settings.clone();
    config::save(&data_dir(), &settings)?;
    Ok(startup_dto(&inner))
}

/// Switches to the default location, creating it if needed.
#[tauri::command]
fn use_default_library(state: State<'_, AppState>, move_existing: bool) -> CmdResult<StartupDto> {
    let target = default_library_path();
    if move_existing {
        return move_library_to(&state, &target);
    }
    // Reuse the shared path handling so the "not empty" and "not a library" rules apply here
    // too rather than being duplicated.
    set_library_path(state, target.display().to_string(), false)
}

/// Opens the folder holding `settings.json`, in case the user wants to inspect it.
#[tauri::command]
fn open_settings_folder(app: tauri::AppHandle) -> CmdResult<()> {
    let dir = data_dir();
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| CmdError::new(format!("could not open the folder: {e}")))
}

// ---------------------------------------------------------------- import

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewFileDto {
    pub source_path: String,
    pub original_name: String,
    pub format_class: String,
    pub format_detail: String,
    pub word_count: Option<i64>,
    pub word_count_is_estimated: bool,
    pub action: FileAction,
    /// True when a file of this format is already stored, so the version choice applies.
    pub needs_version_choice: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewWorkDto {
    /// Stable within this preview; the frontend sends it back to reassign the files.
    pub key: String,
    pub existing_work_id: Option<i64>,
    /// Set when the user chose the work rather than the automatic rules finding it.
    pub manual_target: Option<i64>,
    pub is_new: bool,
    /// True when every file is already stored, so the merge choice cannot change anything.
    pub all_duplicates: bool,
    pub title: String,
    pub author: Option<String>,
    pub external_id: Option<String>,
    pub language: Option<String>,
    pub files: Vec<PreviewFileDto>,
}

/// An existing work the files could be merged into, offered in the preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateDto {
    pub id: i64,
    pub title: String,
    pub author: Option<String>,
    pub external_id: Option<String>,
    /// Formats the work already holds, so the user can see what is there before merging — and
    /// so the interface knows a version choice will be needed.
    pub formats: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSummaryDto {
    pub files_total: usize,
    pub files_to_store: usize,
    pub duplicates: usize,
    pub works_new: usize,
    pub works_existing: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairDto {
    pub from: String,
    pub to: String,
    pub kind: String,
}

/// The plan shown for confirmation before anything is written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportPreviewDto {
    /// Pass this to `execute_import` to apply the plan.
    pub token: String,
    pub works: Vec<PreviewWorkDto>,
    pub pairs: Vec<PairDto>,
    pub pending: Vec<PendingDto>,
    pub summary: ImportSummaryDto,
    /// The version policy the plan was built with, so the dialog can show the default.
    pub keep_both_versions: bool,
    /// Existing works the files could be merged into, for the "并入已有作品" choice. Ranked so
    /// the most likely target comes first, but the user decides.
    pub candidates: Vec<CandidateDto>,
}

/// Builds the preview payload, including the merge candidates.
///
/// Split out because it is rebuilt whenever the user changes a decision, so it has to be a pure
/// function of the library, the paths and the decisions.
fn build_preview(
    lib: &Library,
    token: String,
    paths: &[PathBuf],
    options: ImportOptions,
    manual: &[import::ManualAssignment],
) -> CmdResult<ImportPreviewDto> {
    let preview = import::analyze_with(lib, paths, options, manual)?;
    let keep_both = options.version_preference == VersionPreference::KeepBoth;

    Ok(ImportPreviewDto {
        token,
        works: preview
            .works
            .into_iter()
            .map(|w| PreviewWorkDto {
                key: w.key.clone(),
                existing_work_id: w.existing_work_id,
                manual_target: w.manual_target,
                is_new: w.is_new(),
                all_duplicates: w.is_all_duplicates(),
                title: w.title,
                author: w.author,
                external_id: w.external_id,
                language: w.language,
                files: w
                    .index
                    .into_iter()
                    .map(|f| PreviewFileDto {
                        source_path: f.source_path.display().to_string(),
                        original_name: f.original_name,
                        format_class: f.format.class().as_str().to_string(),
                        format_detail: f.format.detail(),
                        word_count: f.word_count,
                        word_count_is_estimated: f.word_count_is_estimated,
                        needs_version_choice: f.action.needs_version_choice(),
                        action: f.action,
                    })
                    .collect(),
            })
            .collect(),
        pairs: preview
            .relations
            .iter()
            .map(|r| PairDto {
                from: r.from.clone(),
                to: r.to.clone(),
                kind: r.kind.as_str().to_string(),
            })
            .collect(),
        pending: preview
            .pending
            .iter()
            .map(|p| PendingDto {
                external_id: p.external_id.clone(),
                title: p.title.clone(),
                author: p.author.clone(),
                language: p.language.clone(),
                from_title: String::new(),
            })
            .collect(),
        summary: ImportSummaryDto {
            files_total: preview.summary.files_total,
            files_to_store: preview.summary.files_to_store,
            duplicates: preview.summary.duplicates,
            works_new: preview.summary.works_new,
            works_existing: preview.summary.works_existing,
        },
        keep_both_versions: keep_both,
        candidates: merge_candidates(lib)?,
    })
}

/// Existing works offered as merge targets.
///
/// Every work is offered rather than only "similar" ones: judging similarity between a filename
/// and a title is exactly the guess the automatic rules refuse to make — `Zong_Lu_Jian_Dan_Di_Sha.epub`
/// has no textual overlap with `【粽驴】简单地杀个人` — so any shortlist would risk omitting the
/// right answer. The list is capped and ordered newest first, since a file being imported now is
/// usually related to something imported recently.
fn merge_candidates(lib: &Library) -> CmdResult<Vec<CandidateDto>> {
    const LIMIT: usize = 300;

    let mut stmt = lib
        .conn()
        .prepare(
            "SELECT w.id, w.title, w.author, w.external_id,
                    (SELECT GROUP_CONCAT(DISTINCT COALESCE(f.format_detail, ''))
                     FROM files f WHERE f.work_id = w.id)
             FROM works w
             ORDER BY w.updated_at DESC, w.id DESC
             LIMIT ?1",
        )
        .map_err(|e| CmdError::new(format!("could not list existing works: {e}")))?;
    let rows = stmt
        .query_map(rusqlite::params![LIMIT as i64], |r| {
            let formats: Option<String> = r.get(4)?;
            Ok(CandidateDto {
                id: r.get(0)?,
                title: r.get(1)?,
                author: r.get(2)?,
                external_id: r.get(3)?,
                formats: formats
                    .map(|s| s.split(',').map(str::to_owned).collect())
                    .unwrap_or_default(),
            })
        })
        .map_err(|e| CmdError::new(format!("could not list existing works: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CmdError::new(format!("could not list existing works: {e}")))
}

/// Inspects the given paths and returns a plan. Writes nothing.
#[tauri::command]
fn analyze_import(state: State<'_, AppState>, paths: Vec<String>) -> CmdResult<ImportPreviewDto> {
    if paths.is_empty() {
        return Err(CmdError::new("没有选择文件"));
    }
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();

    // The library is only checked here; the plan itself is built afterwards, once the preview
    // token is registered, so that building it and being able to re-build it are the same path.
    let options = {
        let inner = state_lock(&state)?;
        if inner.library.is_none() {
            return Err(CmdError::new(
                inner
                    .library_error
                    .clone()
                    .unwrap_or_else(|| "还没有打开书库".to_owned()),
            ));
        }
        ImportOptions {
            version_preference: if inner.settings.keep_both_versions {
                VersionPreference::KeepBoth
            } else {
                VersionPreference::Replace
            },
        }
    };

    let token = {
        let mut next = state
            .next_token
            .lock()
            .map_err(|_| CmdError::new("internal state was poisoned"))?;
        let token = format!("import-{}", *next);
        *next += 1;
        token
    };
    state_lock(&state)?.pending.insert(
        token.clone(),
        PendingImport {
            paths: paths.clone(),
            options,
            manual: Vec::new(),
        },
    );

    let inner = state_lock(&state)?;
    let lib = inner
        .library
        .as_ref()
        .ok_or_else(|| CmdError::new("还没有打开书库"))?;
    build_preview(lib, token, &paths, options, &[])
}

/// Reassigns the files of one preview work to an existing work, or back to a new one.
///
/// The plan is rebuilt from the stored paths and decisions, so this shows the real consequence
/// immediately: whether the files attach, and whether a version choice will be needed because
/// that work already holds the same format.
#[tauri::command]
fn assign_import_target(
    state: State<'_, AppState>,
    token: String,
    key: String,
    work_id: Option<i64>,
) -> CmdResult<ImportPreviewDto> {
    let (paths, options, manual) = {
        let mut inner = state_lock(&state)?;
        let pending = inner
            .pending
            .get_mut(&token)
            .ok_or_else(|| CmdError::new("这次导入预览已经失效，请重新选择文件"))?;
        pending.manual.retain(|m| m.key != key);
        if let Some(work_id) = work_id {
            pending.manual.push(import::ManualAssignment {
                key: key.clone(),
                work_id,
            });
        }
        (
            pending.paths.clone(),
            pending.options,
            pending.manual.clone(),
        )
    };

    let inner = state_lock(&state)?;
    let lib = inner
        .library
        .as_ref()
        .ok_or_else(|| CmdError::new("还没有打开书库"))?;
    build_preview(lib, token, &paths, options, &manual)
}

/// Applies a plan previously returned by `analyze_import`.
///
/// The plan is rebuilt from the stored paths rather than kept in memory, so a preview cannot be
/// applied against a library that changed underneath it: if the files moved, this fails instead
/// of writing something surprising.
///
/// `keep_both` overrides the stored default for this one import, which is the per-file answer
/// the workflow needs: usually the older revision is worth keeping, occasionally not.
#[tauri::command]
fn execute_import(
    state: State<'_, AppState>,
    token: String,
    keep_both: Option<bool>,
) -> CmdResult<ImportReport> {
    let (paths, mut options, manual) = {
        let mut inner = state_lock(&state)?;
        let pending = inner
            .pending
            .remove(&token)
            .ok_or_else(|| CmdError::new("这次导入预览已经失效，请重新选择文件"))?;
        (pending.paths, pending.options, pending.manual)
    };
    if let Some(keep) = keep_both {
        options.version_preference = if keep {
            VersionPreference::KeepBoth
        } else {
            VersionPreference::Replace
        };
    }

    let inner = state_lock(&state)?;
    let lib = inner
        .library
        .as_ref()
        .ok_or_else(|| CmdError::new("还没有打开书库"))?;
    // Re-planned from the same paths and decisions that produced the preview, so what runs is
    // exactly what was shown.
    let preview = import::analyze_with(lib, &paths, options, &manual)?;
    Ok(import::execute(lib, &preview)?)
}

/// Discards a preview the user cancelled.
#[tauri::command]
fn discard_import(state: State<'_, AppState>, token: String) -> CmdResult<()> {
    state_lock(&state)?.pending.remove(&token);
    Ok(())
}

// ---------------------------------------------------------------- editing

/// A date being edited, as the interface sends it.
#[derive(Debug, Clone, Deserialize)]
pub struct DateEditDto {
    pub input: String,
    #[serde(default)]
    pub approx: bool,
}

/// Saves every editable field of a work in one call.
///
/// One command rather than several so an edit is atomic: a form that changed the title, a date
/// and the tags either saves all of it or none. The frontend only sends fields the user actually
/// touched (`null` means "leave alone"), which is what keeps a partial form from blanking a
/// column — and what gives a date three meanings rather than two: omitted, set, or cleared.
///
/// The parameter list is long because Tauri deserialises command arguments from named IPC
/// fields, so they cannot be nested in a struct without changing the call shape on the frontend.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
fn update_work(
    state: State<'_, AppState>,
    work_id: i64,
    title: Option<String>,
    author: Option<String>,
    summary: Option<String>,
    my_comment: Option<String>,
    published: Option<DateEditDto>,
    completed: Option<DateEditDto>,
    status: Option<String>,
    word_count: Option<i64>,
    language: Option<String>,
    language_label: Option<String>,
    tags: Option<Vec<String>>,
) -> CmdResult<WorkDto> {
    let to_edit = |d: &Option<DateEditDto>| {
        d.as_ref().map(|d| bookmarks_core::store::DateEdit {
            input: d.input.clone(),
            approx: d.approx,
        })
    };

    let edit = bookmarks_core::store::WorkEdit {
        title,
        author,
        summary,
        my_comment,
        published: to_edit(&published),
        completed: to_edit(&completed),
        status,
        language,
        language_label,
        word_count,
        tags,
    };

    with_library(&state, |lib| {
        lib.apply_work_edit(work_id, &edit)?;
        reload_work(lib, work_id)
    })
}

/// Creates an entry with no file, for a work to be recorded before it is downloaded.
#[tauri::command]
fn create_work(
    state: State<'_, AppState>,
    title: String,
    author: Option<String>,
) -> CmdResult<WorkDto> {
    with_library(&state, |lib| {
        let id = lib.insert_record_only_work(&title, author.as_deref())?;
        reload_work(lib, id)
    })
}

/// Copies AO3's own tags into the user's tag list, leaving existing tags in place.
///
/// An explicit action rather than something done at import time: mixing AO3's tag vocabulary
/// with the user's would dilute the user's own filtering.
#[tauri::command]
fn import_ao3_tags(state: State<'_, AppState>, work_id: i64) -> CmdResult<WorkDto> {
    with_library(&state, |lib| {
        let current = view::work_views(lib)?
            .into_iter()
            .find(|w| w.id == work_id)
            .ok_or_else(|| CmdError::new(format!("no work with id {work_id}")))?;

        let mut merged = current.tags.clone();
        for tag in &current.ao3_tags {
            if !merged.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                merged.push(tag.clone());
            }
        }
        lib.set_work_tags(work_id, &merged)?;
        reload_work(lib, work_id)
    })
}

fn reload_work(lib: &Library, work_id: i64) -> CmdResult<WorkDto> {
    view::work_views(lib)?
        .into_iter()
        .find(|w| w.id == work_id)
        .map(WorkDto::from)
        .ok_or_else(|| CmdError::new(format!("work {work_id} is no longer in the library")))
}

/// Marks a suggested translation as not wanted, so it stops being reported.
///
/// Recorded rather than deleted, so the decision can be undone and so re-importing the original
/// does not bring the suggestion back.
#[tauri::command]
fn dismiss_pending(state: State<'_, AppState>, external_id: String) -> CmdResult<()> {
    with_library(&state, |lib| {
        lib.dismiss_pending(&external_id)?;
        Ok(())
    })
}

/// Marks every outstanding translation suggestion as not wanted.
#[tauri::command]
fn dismiss_all_pending(state: State<'_, AppState>) -> CmdResult<usize> {
    with_library(&state, |lib| Ok(lib.dismiss_all_pending()?))
}

/// Undoes [`dismiss_pending`].
#[tauri::command]
fn restore_pending(state: State<'_, AppState>, external_id: String) -> CmdResult<()> {
    with_library(&state, |lib| {
        lib.restore_pending(&external_id)?;
        Ok(())
    })
}

/// Suggestions the user dismissed, so they can be restored from the settings panel.
#[tauri::command]
fn list_dismissed_pending(state: State<'_, AppState>) -> CmdResult<Vec<PendingDto>> {
    with_library(&state, |lib| {
        Ok(lib
            .dismissed_pending()?
            .into_iter()
            .map(|d| PendingDto {
                external_id: d.external_id,
                title: d.title,
                author: d.author,
                language: None,
                from_title: String::new(),
            })
            .collect())
    })
}

/// Replaces a work's series membership.
#[tauri::command]
fn set_work_series(
    state: State<'_, AppState>,
    work_id: i64,
    series: Vec<SeriesMembershipDto>,
) -> CmdResult<WorkDto> {
    let memberships: Vec<SeriesMembership> = series
        .into_iter()
        .map(|s| SeriesMembership {
            name: s.name,
            position: s.position,
        })
        .collect();
    with_library(&state, |lib| {
        lib.set_work_series(work_id, &memberships)?;
        reload_work(lib, work_id)
    })
}

/// A work's membership in one series, as the interface sends it.
#[derive(Debug, Clone, Deserialize)]
pub struct SeriesMembershipDto {
    pub name: String,
    #[serde(default)]
    pub position: Option<i64>,
}

// ---------------------------------------------------------------- files

/// Opens a stored file with the system's default application.
///
/// This is why no in-app reader is needed: PDF and EPUB rendering is left to whatever the user
/// already has installed.
#[tauri::command]
fn open_file(app: tauri::AppHandle, state: State<'_, AppState>, file_id: i64) -> CmdResult<()> {
    let abs = with_library(&state, |lib| resolve_stored_file(lib, file_id))?;
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(abs.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| CmdError::new(format!("could not open the file: {e}")))
}

/// Reveals a stored file in the system file manager.
#[tauri::command]
fn reveal_file(app: tauri::AppHandle, state: State<'_, AppState>, file_id: i64) -> CmdResult<()> {
    let abs = with_library(&state, |lib| resolve_stored_file(lib, file_id))?;
    tauri_plugin_opener::OpenerExt::opener(&app)
        .reveal_item_in_dir(&abs)
        .map_err(|e| CmdError::new(format!("could not open the folder: {e}")))
}

/// Opens the library folder itself, so it can be backed up by copying it.
#[tauri::command]
fn open_library_folder(app: tauri::AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let root = with_library(&state, |lib| Ok(lib.root().to_path_buf()))?;
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(root.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| CmdError::new(format!("could not open the library folder: {e}")))
}

/// Promotes a stored file to be the current version of its format.
#[tauri::command]
fn set_current_version(state: State<'_, AppState>, file_id: i64) -> CmdResult<WorkDto> {
    with_library(&state, |lib| {
        let work_id = work_id_of_file(lib, file_id)?;
        lib.set_current_version(file_id)?;
        reload_work(lib, work_id)
    })
}

/// Deletes an archived version. Refuses to delete the current one.
#[tauri::command]
fn delete_version(state: State<'_, AppState>, file_id: i64) -> CmdResult<WorkDto> {
    with_library(&state, |lib| {
        let work_id = work_id_of_file(lib, file_id)?;
        lib.delete_version(file_id)?;
        reload_work(lib, work_id)
    })
}

/// Deletes a work, its files and its directory.
///
/// Reports how many files went with it, so the interface can say what actually happened rather
/// than just closing.
#[tauri::command]
fn delete_work(state: State<'_, AppState>, work_id: i64) -> CmdResult<usize> {
    with_library(&state, |lib| {
        let removed = lib.files_for_work(work_id)?.len();
        lib.delete_work(work_id)?;
        Ok(removed)
    })
}

fn work_id_of_file(lib: &Library, file_id: i64) -> CmdResult<i64> {
    lib.conn()
        .query_row(
            "SELECT work_id FROM files WHERE id = ?1",
            rusqlite::params![file_id],
            |r| r.get(0),
        )
        .map_err(|_| CmdError::new(format!("no file with id {file_id}")))
}

/// Looks up a stored file and confirms it is present and inside the library.
fn resolve_stored_file(lib: &Library, file_id: i64) -> CmdResult<PathBuf> {
    let rel: String = lib
        .conn()
        .query_row(
            "SELECT rel_path FROM files WHERE id = ?1",
            rusqlite::params![file_id],
            |r| r.get(0),
        )
        .map_err(|_| CmdError::new(format!("no file with id {file_id}")))?;

    let abs = lib.abs_path(&rel);
    if !abs.is_file() {
        return Err(CmdError::new(format!(
            "文件在书库里找不到了：{}",
            abs.display()
        )));
    }
    // The path comes from the database, but confirm it is inside the library before handing it
    // to the shell: a tampered row must not be able to launch anything.
    assert_inside_library(lib.root(), &abs)?;
    Ok(abs)
}

/// Refuses a path that is not inside the library root.
fn assert_inside_library(root: &Path, candidate: &Path) -> CmdResult<()> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let candidate = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf());
    if !candidate.starts_with(&root) {
        return Err(CmdError::new("refusing to open a path outside the library"));
    }
    Ok(())
}

/// Reads a text file the user picked, decoding it. Used for the AO3 CSV bookmarklet export.
#[tauri::command]
fn read_text_file(path: String) -> CmdResult<String> {
    let bytes = std::fs::read(&path)?;
    Ok(bookmarks_core::fileformat::decode_text(&bytes))
}

// ---------------------------------------------------------------- entry point

/// Builds and runs the application.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Settings are read first, because they hold the library location. Startup cannot
            // fail: a missing or unreadable library is a state the window renders, not a crash.
            app.manage(AppState::new(data_dir()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            startup,
            load_library,
            save_settings,
            set_library_path,
            use_default_library,
            open_settings_folder,
            analyze_import,
            assign_import_target,
            execute_import,
            discard_import,
            update_work,
            create_work,
            import_ao3_tags,
            dismiss_pending,
            dismiss_all_pending,
            restore_pending,
            list_dismissed_pending,
            set_work_series,
            set_current_version,
            delete_version,
            delete_work,
            open_file,
            reveal_file,
            open_library_folder,
            read_text_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_data_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("bookmarks-app-tests")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn the_default_library_is_outside_the_settings_directory() {
        let def = default_library_path();
        assert!(
            !def.starts_with(data_dir()),
            "the library must not live inside the settings directory: {}",
            def.display()
        );
        assert!(
            def.to_string_lossy().contains("bookmarks"),
            "unexpected default: {}",
            def.display()
        );
    }

    /// A hard-coded `D:\...` path would create a library on a drive that may not exist on
    /// someone else's machine, so the default must be derived from the user's profile.
    #[test]
    fn the_default_library_is_derived_from_the_user_profile_not_hard_coded() {
        let def = default_library_path();

        // Only meaningful when the override is not in play; the variable is set in some CI runs
        // to keep tests off a real library, and then this test has nothing to say.
        if std::env::var_os("BOOKMARKS_LIBRARY").is_some() {
            return;
        }

        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from);
        if let Some(home) = home {
            assert!(
                def.starts_with(&home),
                "the default should sit under the user's profile, got {}",
                def.display()
            );
        }
    }

    #[test]
    fn settings_and_the_library_are_separate_locations() {
        let data = temp_data_dir("separate");
        let lib = data.join("elsewhere");
        let settings = Settings {
            library_path: Some(lib.display().to_string()),
            ..Default::default()
        };
        config::save(&data, &settings).unwrap();

        assert!(config::settings_file(&data).is_file());
        assert_eq!(
            config::load(&data).library_path,
            Some(lib.display().to_string())
        );
        // Writing settings must not create the library directory.
        assert!(!lib.exists(), "saving settings must not touch the library");
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_fresh_library_opens_at_the_latest_version() {
        let data = temp_data_dir("fresh");
        let db = bookmarks_core::db::default_path(data.join("lib"));
        let conn = bookmarks_core::db::open(&db).expect("open library");
        assert_eq!(
            bookmarks_core::db::schema_version(&conn).unwrap(),
            bookmarks_core::db::latest_version()
        );
        drop(conn);
        assert_eq!(
            inspect_database_version(&db).unwrap(),
            bookmarks_core::db::latest_version()
        );
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn inspecting_a_non_library_file_reports_version_zero() {
        let data = temp_data_dir("notalibrary");
        let db = bookmarks_core::db::default_path(&data);
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE unrelated (x INTEGER);")
            .unwrap();
        drop(conn);
        assert_eq!(inspect_database_version(&db).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn refusing_paths_outside_the_library() {
        let data = temp_data_dir("outside");
        let root = data.join("lib");
        let inside = root.join("library").join("000001");
        std::fs::create_dir_all(&inside).unwrap();
        let inside_file = inside.join("work.html");
        std::fs::write(&inside_file, b"x").unwrap();
        assert!(assert_inside_library(&root, &inside_file).is_ok());

        let outside = data.join("elsewhere.txt");
        std::fs::write(&outside, b"x").unwrap();
        assert!(assert_inside_library(&root, &outside).is_err());

        let traversal = inside
            .join("..")
            .join("..")
            .join("..")
            .join("elsewhere.txt");
        assert!(assert_inside_library(&root, &traversal).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn resolving_a_stored_file_rejects_unknown_ids_and_missing_files() {
        let data = temp_data_dir("resolve");
        let lib = open_library_at(&data.join("lib")).unwrap();
        assert!(resolve_stored_file(&lib, 4242).is_err(), "unknown id");

        let work = lib
            .insert_work(&bookmarks_core::store::NewWork {
                title: "T".into(),
                ..Default::default()
            })
            .unwrap();
        let src = data.join("a.html");
        std::fs::write(&src, b"<html>x</html>").unwrap();
        let rec = lib
            .store_file(
                work,
                &src,
                &bookmarks_core::fileformat::FileFormat::Html,
                None,
                false,
                bookmarks_core::store::FileVersionPolicy::ReplaceOnly,
            )
            .unwrap()
            .into_file()
            .unwrap();

        assert!(resolve_stored_file(&lib, rec.id).is_ok());
        std::fs::remove_file(lib.abs_path(&rec.rel_path)).unwrap();
        assert!(
            resolve_stored_file(&lib, rec.id).is_err(),
            "a file missing from disk must be reported, not opened"
        );
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn counts_dto_maps_from_the_view() {
        let c = view::LibraryCounts {
            works_total: 87,
            works_with_file: 84,
            works_record_only: 3,
            files_total: 126,
            works_related: 5,
            pending_relations: 2,
        };
        let dto: CountsDto = c.into();
        assert_eq!(dto.works_total, 87);
        assert_eq!(dto.files_total, 126);
        assert_eq!(dto.pending_relations, 2);
    }
}
