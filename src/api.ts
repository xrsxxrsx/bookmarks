/**
 * The only place that talks to the Rust side.
 *
 * Every command name and argument shape appears here, so the boundary is reviewable in one
 * file. Errors from Tauri arrive as `{ message }` and are rethrown as `Error` with that
 * text, so callers can show them directly.
 */
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

import type {
  ImportPreview,
  ImportReport,
  Library,
  Settings,
  Startup,
  Work,
  WorkUpdate,
} from './types';

/** Turns Tauri's error payload into a normal `Error`. */
function toError(e: unknown): Error {
  if (e instanceof Error) return e;
  if (typeof e === 'object' && e !== null && 'message' in e) {
    return new Error(String((e as { message: unknown }).message));
  }
  return new Error(String(e));
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (e) {
    throw toError(e);
  }
}

/** Reads settings and reports whether a library is open. */
export function startup(): Promise<Startup> {
  return call<Startup>('startup');
}

/** Reads the whole library. The interface filters, sorts and groups it in memory. */
export function loadLibrary(): Promise<Library> {
  return call<Library>('load_library');
}

/** Saves one or more interface settings. */
export function saveSettings(patch: Partial<Settings>): Promise<Settings> {
  return call<Settings>('save_settings', {
    theme: patch.theme ?? null,
    fontSizePx: patch.font_size_px ?? null,
    groupMode: patch.group_mode ?? null,
    keepBothVersions: patch.keep_both_versions ?? null,
  });
}

/**
 * Points the application at a library, optionally moving the current one there.
 *
 * `moveExisting` is the destructive option, so it is never the default: without it the target
 * must already be a library or be empty.
 */
export function setLibraryPath(path: string, moveExisting: boolean): Promise<Startup> {
  return call<Startup>('set_library_path', { path, moveExisting });
}

/** Switches to the default location, creating it if needed. */
export function useDefaultLibrary(moveExisting: boolean): Promise<Startup> {
  return call<Startup>('use_default_library', { moveExisting });
}

/** Opens the folder holding `settings.json`. */
export function openSettingsFolder(): Promise<void> {
  return call<void>('open_settings_folder');
}

/** Picks a folder to use as the library location. Returns null when the user cancels. */
export async function pickLibraryFolder(title: string): Promise<string | null> {
  const chosen = await open({ multiple: false, directory: true, title });
  if (chosen === null) return null;
  return Array.isArray(chosen) ? (chosen[0] ?? null) : chosen;
}

/**
 * Shows the file picker and returns the chosen paths. Returns an empty array when the
 * user cancels, which is not an error.
 */
export async function pickFiles(): Promise<string[]> {
  const chosen = await open({
    multiple: true,
    directory: false,
    title: '选择要导入的文件',
  });
  if (chosen === null) return [];
  return Array.isArray(chosen) ? chosen : [chosen];
}

/** Shows the folder picker; a folder is scanned recursively for importable files. */
export async function pickFolder(): Promise<string[]> {
  const chosen = await open({
    multiple: false,
    directory: true,
    title: '选择要导入的文件夹',
  });
  if (chosen === null) return [];
  return Array.isArray(chosen) ? chosen : [chosen];
}

/** Inspects files and returns a plan. Nothing is written. */
export function analyzeImport(paths: string[]): Promise<ImportPreview> {
  return call<ImportPreview>('analyze_import', { paths });
}

/** Applies a plan previously returned by {@link analyzeImport}. */
export function executeImport(token: string, keepBoth?: boolean): Promise<ImportReport> {
  return call<ImportReport>('execute_import', { token, keepBoth: keepBoth ?? null });
}

/**
 * Reassigns the files of one preview work to an existing work, or back to a new one.
 *
 * The plan comes back rebuilt, so the interface shows the real consequence immediately —
 * including whether a version choice will now be needed.
 */
export function assignImportTarget(
  token: string,
  key: string,
  workId: number | null,
): Promise<ImportPreview> {
  return call<ImportPreview>('assign_import_target', { token, key, workId });
}

/** Discards a preview the user cancelled, so its token cannot be reused. */
export function discardImport(token: string): Promise<void> {
  return call<void>('discard_import', { token });
}

/** Saves edits and returns the updated work. */
export function updateWork(update: WorkUpdate): Promise<Work> {
  return call<Work>('update_work', {
    workId: update.work_id,
    title: update.title ?? null,
    author: update.author ?? null,
    summary: update.summary ?? null,
    myComment: update.my_comment ?? null,
    published: update.published ?? null,
    completed: update.completed ?? null,
    status: update.status ?? null,
    wordCount: update.word_count ?? null,
    language: update.language ?? null,
    languageLabel: update.language_label ?? null,
    tags: update.tags ?? null,
  });
}

/**
 * Creates an entry with no file.
 *
 * Recorded as `record_only`, which is how a bookmark whose file was never downloaded stays
 * visible in the library.
 */
export function createWork(title: string, author: string | null): Promise<Work> {
  return call<Work>('create_work', { title, author });
}

/** Copies AO3's own tags into the user's tag list, keeping existing ones. */
export function importAo3Tags(workId: number): Promise<Work> {
  return call<Work>('import_ao3_tags', { workId });
}

/**
 * Deletes a work, its files and its directory.
 *
 * Returns how many files went with it. This cannot be undone from inside the application, so the
 * interface asks for confirmation first.
 */
export function deleteWork(workId: number): Promise<number> {
  return call<number>('delete_work', { workId });
}

/** Promotes a stored file to be the current version of its format. */
export function setCurrentVersion(fileId: number): Promise<Work> {
  return call<Work>('set_current_version', { fileId });
}

/** Deletes an archived version. The current version cannot be deleted this way. */
export function deleteVersion(fileId: number): Promise<Work> {
  return call<Work>('delete_version', { fileId });
}

/** Replaces a work's series membership. */
export function setWorkSeries(
  workId: number,
  series: { name: string; position: number | null }[],
): Promise<Work> {
  return call<Work>('set_work_series', { workId, series });
}

/** Marks a suggested translation as not wanted, so it stops being reported. */
export function dismissPending(externalId: string): Promise<void> {
  return call<void>('dismiss_pending', { externalId });
}

/** Marks every outstanding suggestion as not wanted. Returns how many were cleared. */
export function dismissAllPending(): Promise<number> {
  return call<number>('dismiss_all_pending');
}

/** Undoes {@link dismissPending}. */
export function restorePending(externalId: string): Promise<void> {
  return call<void>('restore_pending', { externalId });
}

/** Suggestions the user dismissed, so they can be restored. */
export function listDismissedPending(): Promise<
  { external_id: string; title: string | null; author: string | null }[]
> {
  return call('list_dismissed_pending');
}

/** Opens a stored file with the system's default application. */
export function openFile(fileId: number): Promise<void> {
  return call<void>('open_file', { fileId });
}

/** Reveals a stored file in the system file manager. */
export function revealFile(fileId: number): Promise<void> {
  return call<void>('reveal_file', { fileId });
}

/** Opens the library folder, so it can be backed up by copying it. */
export function openLibraryFolder(): Promise<void> {
  return call<void>('open_library_folder');
}
