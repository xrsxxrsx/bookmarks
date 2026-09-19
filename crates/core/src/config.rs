//! Application settings.
//!
//! Settings live in a small JSON file **outside** the library, in the application data
//! directory. That placement is forced by a circular dependency: the library's location is
//! itself a setting, so it cannot be stored inside the library. The file is a few hundred
//! bytes and stays in `%APPDATA%`; the database and every imported file live wherever the
//! user points `library_path`.
//!
//! Nothing here fails hard. A missing, unreadable or malformed settings file falls back to
//! defaults, because refusing to start over a settings file would be worse than losing a
//! preference, and the file is the one thing a user might edit by hand.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Whether a related work is grouped under its partner, and by which relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupMode {
    /// Group a work with its translations.
    #[default]
    Translation,
    /// Group a work with the rest of its series.
    Series,
    /// No grouping: a strictly flat list.
    None,
}

impl GroupMode {
    pub fn as_str(self) -> &'static str {
        match self {
            GroupMode::Translation => "translation",
            GroupMode::Series => "series",
            GroupMode::None => "none",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "translation" => Some(GroupMode::Translation),
            "series" => Some(GroupMode::Series),
            "none" => Some(GroupMode::None),
            _ => None,
        }
    }
}

/// Light is the default: the requested accent is a saturated blue, which only reads as vivid on
/// a light background (8.6:1 there, against 2.2:1 on dark).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }
}

/// Root font size in pixels. The whole interface is sized in `rem`, so this scales all of it.
pub const FONT_SIZE_CHOICES: [u16; 4] = [13, 14, 16, 18];
const DEFAULT_FONT_SIZE: u16 = 14;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Absolute path of the library root: the directory containing `bookmarks.db` and
    /// `library/`.
    pub library_path: Option<String>,
    pub theme: Theme,
    pub font_size_px: u16,
    pub group_mode: GroupMode,
    /// What to do when a file of the same format is already stored for a work.
    pub keep_both_versions: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            library_path: None,
            theme: Theme::default(),
            font_size_px: DEFAULT_FONT_SIZE,
            group_mode: GroupMode::default(),
            // Keeping the earlier revision is the safer default: a re-import usually means
            // the work was updated, and a lost revision is worse than an extra copy.
            keep_both_versions: true,
        }
    }
}

impl Settings {
    /// Clamps values that a hand-edited file could have got wrong, so the interface never has
    /// to defend against them.
    pub fn sanitised(mut self) -> Self {
        if !FONT_SIZE_CHOICES.contains(&self.font_size_px) {
            self.font_size_px = DEFAULT_FONT_SIZE;
        }
        if self
            .library_path
            .as_deref()
            .is_some_and(|p| p.trim().is_empty())
        {
            self.library_path = None;
        }
        self
    }
}

/// Path of the settings file inside an application data directory.
pub fn settings_file(data_dir: &Path) -> PathBuf {
    data_dir.join("settings.json")
}

/// Reads settings, falling back to defaults in every failure case.
pub fn load(data_dir: &Path) -> Settings {
    let path = settings_file(data_dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<Settings>(&text)
            .map(Settings::sanitised)
            .unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

/// Writes settings, creating the directory if needed.
pub fn save(data_dir: &Path, settings: &Settings) -> Result<()> {
    std::fs::create_dir_all(data_dir).map_err(|e| Error::io(data_dir, e))?;
    let path = settings_file(data_dir);
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| Error::Invalid(format!("could not encode settings: {e}")))?;
    // Written via a temporary file so an interrupted save cannot leave a truncated
    // settings file that then silently falls back to defaults.
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text).map_err(|e| Error::io(&temp, e))?;
    std::fs::rename(&temp, &path).map_err(|e| Error::io(&path, e))?;
    Ok(())
}

/// What a directory contains, as far as a library is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Look {
    /// Does not exist, or exists but is empty.
    Empty,
    /// Looks like a library: has the database or the file store.
    IsLibrary,
    /// Contains files, none of which are a library. Moving a library here would mix it in.
    OccupiedByOther,
}

/// Inspects a candidate library directory.
pub fn inspect(dir: &Path) -> Look {
    if !dir.exists() {
        return Look::Empty;
    }
    if dir.join("bookmarks.db").is_file() || dir.join("library").is_dir() {
        return Look::IsLibrary;
    }
    let mut entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Look::Empty,
    };
    if entries.next().is_none() {
        Look::Empty
    } else {
        Look::OccupiedByOther
    }
}

/// Moves a library to a new directory.
///
/// Copied to a staging directory inside the destination, verified, and only then swapped into
/// place and the source removed. A failure part-way therefore leaves the original untouched
/// rather than half-moved — the one outcome that would actually cost data.
///
/// `verify` receives the number of bytes copied and is used for progress reporting.
pub fn move_library(from: &Path, to: &Path, mut verify: impl FnMut(u64)) -> Result<()> {
    if from == to {
        return Ok(());
    }
    if !from.exists() {
        return Err(Error::Invalid(format!(
            "the library at {} does not exist",
            from.display()
        )));
    }
    // Never write into a directory that already holds something else.
    match inspect(to) {
        Look::Empty => {}
        Look::IsLibrary => {
            return Err(Error::Invalid(format!(
                "{} already contains a library",
                to.display()
            )))
        }
        Look::OccupiedByOther => {
            return Err(Error::Invalid(format!(
                "{} is not empty and does not look like a library",
                to.display()
            )))
        }
    }

    let staging = to.with_file_name(format!(
        "{}.moving-{}",
        to.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "library".to_owned()),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| Error::io(&staging, e))?;

    if let Err(e) = copy_tree(from, &staging, &mut verify) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }

    // Verify before destroying anything: the copy must contain the database and as many
    // files as the source, or the move is abandoned.
    let source_files = count_files(from);
    let copied_files = count_files(&staging);
    if !staging.join("bookmarks.db").is_file() || copied_files != source_files {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(Error::Invalid(format!(
            "copy verification failed: {copied_files} file(s) copied, expected {source_files}"
        )));
    }

    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    if let Err(e) = std::fs::rename(&staging, to) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(Error::io(to, e));
    }

    // The library is safely in its new home; the old copy can go.
    std::fs::remove_dir_all(from).map_err(|e| Error::io(from, e))?;
    Ok(())
}

fn count_files(dir: &Path) -> u64 {
    let mut total = 0;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => total += count_files(&path),
            Ok(t) if t.is_file() => total += 1,
            _ => {}
        }
    }
    total
}

fn copy_tree(from: &Path, to: &Path, verify: &mut impl FnMut(u64)) -> Result<()> {
    std::fs::create_dir_all(to).map_err(|e| Error::io(to, e))?;
    let entries = std::fs::read_dir(from).map_err(|e| Error::io(from, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(from, e))?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        let file_type = entry.file_type().map_err(|e| Error::io(&source, e))?;
        if file_type.is_dir() {
            copy_tree(&source, &target, verify)?;
        } else if file_type.is_file() {
            let bytes = std::fs::copy(&source, &target).map_err(|e| Error::io(&source, e))?;
            verify(bytes);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("bookmarks-config-tests")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_settings_file_yields_defaults() {
        let data = temp("missing");
        let s = load(&data);
        assert_eq!(s, Settings::default());
        assert_eq!(s.theme, Theme::Light, "light is the default");
        assert!(
            s.keep_both_versions,
            "keeping both versions is the safer default"
        );
        assert_eq!(s.group_mode, GroupMode::Translation);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn settings_round_trip() {
        let data = temp("roundtrip");
        let settings = Settings {
            library_path: Some(r"D:\books".to_owned()),
            theme: Theme::Dark,
            font_size_px: 16,
            group_mode: GroupMode::Series,
            keep_both_versions: false,
        };
        save(&data, &settings).unwrap();
        assert_eq!(load(&data), settings);
        // The file is real JSON a user could edit.
        let text = std::fs::read_to_string(settings_file(&data)).unwrap();
        assert!(text.contains("\"library_path\""));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_corrupt_settings_file_falls_back_instead_of_failing_to_start() {
        let data = temp("corrupt");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(settings_file(&data), "{ this is not json").unwrap();
        assert_eq!(load(&data), Settings::default());

        // Also fine when it is valid JSON of the wrong shape.
        std::fs::write(settings_file(&data), "[1,2,3]").unwrap();
        assert_eq!(load(&data), Settings::default());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_partial_settings_file_keeps_defaults_for_absent_fields() {
        let data = temp("partial");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(settings_file(&data), r#"{"theme":"dark"}"#).unwrap();
        let s = load(&data);
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.font_size_px, 14, "an absent field keeps its default");
        assert!(s.keep_both_versions);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_hand_edited_out_of_range_font_size_is_corrected() {
        let data = temp("badfont");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(settings_file(&data), r#"{"font_size_px":400}"#).unwrap();
        assert_eq!(load(&data).font_size_px, 14);

        std::fs::write(settings_file(&data), r#"{"font_size_px":16}"#).unwrap();
        assert_eq!(load(&data).font_size_px, 16);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn an_empty_library_path_is_treated_as_unset() {
        let data = temp("emptypath");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(settings_file(&data), r#"{"library_path":"   "}"#).unwrap();
        assert_eq!(load(&data).library_path, None);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn inspecting_a_directory_classifies_it() {
        let dir = temp("inspect");
        assert_eq!(inspect(&dir.join("nope")), Look::Empty);
        assert_eq!(inspect(&dir), Look::Empty, "an empty directory is usable");

        // A real library.
        let lib = dir.join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("bookmarks.db"), b"x").unwrap();
        assert_eq!(inspect(&lib), Look::IsLibrary);

        // A library recognised by its file store alone.
        let lib2 = dir.join("lib2");
        std::fs::create_dir_all(lib2.join("library")).unwrap();
        assert_eq!(inspect(&lib2), Look::IsLibrary);

        // Something else entirely.
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("holiday.jpg"), b"x").unwrap();
        assert_eq!(inspect(&other), Look::OccupiedByOther);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn moving_a_library_carries_every_file_across() {
        let dir = temp("move");
        let from = dir.join("old");
        std::fs::create_dir_all(from.join("library").join("000001")).unwrap();
        std::fs::write(from.join("bookmarks.db"), b"database").unwrap();
        std::fs::write(
            from.join("library").join("000001").join("work.html"),
            b"<html>x</html>",
        )
        .unwrap();
        std::fs::write(from.join("library").join("000001").join("work.epub"), b"PK").unwrap();

        let to = dir.join("new");
        let mut copied = 0u64;
        move_library(&from, &to, |n| copied += n).unwrap();

        assert!(to.join("bookmarks.db").is_file(), "the database moved");
        assert!(to
            .join("library")
            .join("000001")
            .join("work.html")
            .is_file());
        assert!(to
            .join("library")
            .join("000001")
            .join("work.epub")
            .is_file());
        assert!(!from.exists(), "the old location is gone");
        assert!(copied > 0, "progress was reported");
        // No staging directory left behind.
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("moving"))
            .collect();
        assert!(leftovers.is_empty(), "staging left behind: {leftovers:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn moving_refuses_a_destination_that_already_holds_data() {
        let dir = temp("conflict");
        let from = dir.join("old");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("bookmarks.db"), b"mine").unwrap();

        // Another library.
        let other_lib = dir.join("otherlib");
        std::fs::create_dir_all(&other_lib).unwrap();
        std::fs::write(other_lib.join("bookmarks.db"), b"someone else's").unwrap();
        let err = move_library(&from, &other_lib, |_| {}).unwrap_err();
        assert!(
            err.to_string().contains("already contains a library"),
            "got: {err}"
        );
        assert!(
            from.exists(),
            "the source must be untouched after a refusal"
        );
        assert_eq!(
            std::fs::read(other_lib.join("bookmarks.db")).unwrap(),
            b"someone else's",
            "the other library must not be overwritten"
        );

        // An ordinary directory with unrelated files.
        let occupied = dir.join("photos");
        std::fs::create_dir_all(&occupied).unwrap();
        std::fs::write(occupied.join("a.jpg"), b"x").unwrap();
        let err = move_library(&from, &occupied, |_| {}).unwrap_err();
        assert!(err.to_string().contains("not empty"), "got: {err}");
        assert!(from.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn moving_to_the_same_path_is_a_no_op() {
        let dir = temp("same");
        let from = dir.join("lib");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("bookmarks.db"), b"x").unwrap();
        move_library(&from, &from, |_| {}).unwrap();
        assert!(from.join("bookmarks.db").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn moving_a_library_that_does_not_exist_is_an_error() {
        let dir = temp("nosource");
        let err = move_library(&dir.join("nothing"), &dir.join("dest"), |_| {}).unwrap_err();
        assert!(err.to_string().contains("does not exist"), "got: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn theme_and_group_mode_serialise_as_readable_words() {
        assert_eq!(Theme::Light.as_str(), "light");
        assert_eq!(GroupMode::Series.as_str(), "series");
        assert_eq!(Theme::parse("dark"), Some(Theme::Dark));
        assert_eq!(GroupMode::parse("series"), Some(GroupMode::Series));
        assert_eq!(GroupMode::parse("nonsense"), None);
    }
}
