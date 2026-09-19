/**
 * Types mirroring the Rust DTOs in `crates/app/src/lib.rs`.
 *
 * These are hand-written rather than generated so the contract is visible in one place.
 * They must be kept in step with the Rust structs; the field names match exactly, because
 * the DTOs are serialised with their Rust names.
 */

export interface Counts {
  works_total: number;
  works_with_file: number;
  works_record_only: number;
  files_total: number;
  works_related: number;
  pending_relations: number;
}

export interface FileEntry {
  id: number;
  /** Coarse class: `pdf` | `html` | `other`. */
  format_class: 'pdf' | 'html' | 'other' | string;
  /** The real extension, shown alongside the class as "Other · EPUB". */
  format_detail: string;
  original_name: string;
  rel_path: string;
  size_bytes: number;
  word_count: number | null;
  is_estimated: boolean;
  /** False when the file has been deleted from the library folder behind the app's back. */
  exists: boolean;
}

export interface RelatedWork {
  work_id: number;
  title: string;
  author: string | null;
  language: string | null;
  /** `translation` | `remix` | `related` */
  kind: string;
  /** `ao3_html` when AO3 stated it, `manual` when the user linked it. */
  origin: string;
}

/** A series a work belongs to. */
export interface WorkSeries {
  series_id: number;
  name: string;
  /** Position within the series. Null when membership is known but the order is not. */
  position: number | null;
  /** How many works the series holds, so the editor can say "3 篇". */
  total: number;
}

export interface Work {
  id: number;
  title: string;
  author: string | null;
  summary: string | null;
  my_comment: string | null;
  published_at: string | null;
  published_prec: string | null;
  date_is_approx: boolean;
  /** Already formatted for display: `2019`, `~2019`, `2019-06-09`. */
  published_display: string | null;
  completed_at: string | null;
  completed_prec: string | null;
  completed_is_approx: boolean;
  /** Already formatted for display. */
  completed_display: string | null;
  word_count: number;
  word_count_source: string | null;
  /** Already formatted: `67,851`, `~23,159`, `—`. */
  word_count_display: string;
  language: string | null;
  language_label: string | null;
  translation_role: string | null;
  source_url: string | null;
  external_id: string | null;
  ao3_tags: string[];
  tags: string[];
  /** `has_file` | `record_only` */
  status: string;
  files: FileEntry[];
  related: RelatedWork[];
  series: WorkSeries[];
  /** Folded with the same rules the backend uses, so matching needs no reimplementation. */
  search_text: string;
}

export interface PendingRelation {
  external_id: string;
  title: string | null;
  author: string | null;
  language: string | null;
  from_title: string;
}

export interface Library {
  works: Work[];
  pending: PendingRelation[];
  all_tags: string[];
  counts: Counts;
  library_root: string;
  /** Every series name in use, for the editor's suggestions. */
  all_series: string[];
}

/** What the importer intends to do with one file. */
export type FileAction =
  | { kind: 'create_work' }
  | { kind: 'attach_to_existing' }
  | { kind: 'skipped_duplicate'; existing_work_id: number }
  | { kind: 'replaces_stored_file'; existing_work_id: number }
  | { kind: 'keep_both_versions'; existing_work_id: number };

export interface PreviewFile {
  source_path: string;
  original_name: string;
  format_class: string;
  format_detail: string;
  word_count: number | null;
  word_count_is_estimated: boolean;
  action: FileAction;
  /** True when a file of this format is already stored, so the version choice applies. */
  needs_version_choice: boolean;
}

export interface PreviewWork {
  /** Stable within this preview; sent back to reassign the files. */
  key: string;
  existing_work_id: number | null;
  /** Set when the user chose the work rather than the automatic rules finding it. */
  manual_target: number | null;
  is_new: boolean;
  /** True when every file is already stored, so the merge choice cannot change anything. */
  all_duplicates: boolean;
  title: string;
  author: string | null;
  external_id: string | null;
  language: string | null;
  files: PreviewFile[];
}

/** An existing work the imported files could be merged into. */
export interface Candidate {
  id: number;
  title: string;
  author: string | null;
  external_id: string | null;
  /** Formats the work already holds, so the consequence of merging is visible up front. */
  formats: string[];
}

export interface FontSettings {
  theme: string;
  font_size_px: number;
  group_mode: string;
  keep_both_versions: boolean;
}

export interface Settings extends FontSettings {
  library_path: string;
  default_library_path: string;
  settings_file: string;
  font_size_choices: number[];
}

/** What the window needs before it can render a library. */
export interface Startup {
  /** `open` | `unconfigured` | `error` */
  state: string;
  library_path: string;
  default_library_path: string;
  error: string | null;
  settings: Settings;
}

export interface ImportSummary {
  files_total: number;
  files_to_store: number;
  duplicates: number;
  works_new: number;
  works_existing: number;
}

export interface Pair {
  from: string;
  to: string;
  kind: string;
}

export interface ImportPreview {
  token: string;
  works: PreviewWork[];
  pairs: Pair[];
  pending: PendingRelation[];
  summary: ImportSummary;
  /** The version policy the plan was built with, so the dialog can show the default. */
  keep_both_versions: boolean;
  /** Existing works offered as merge targets. */
  candidates: Candidate[];
}

export interface ImportReport {
  works_created: number[];
  works_matched: number[];
  files_stored: number;
  files_skipped: number;
  stored_paths: string[];
  relations_stored: number;
  relations_pending: number;
  errors: [string, string][];
}

/** A date being set from the interface. */
export interface DateEdit {
  /** What the user typed: `2019`, `2019-06` or `2019-06-09`. Precision is derived. */
  input: string;
  /** The value is a guess rather than something a source stated. */
  approx: boolean;
}

export interface WorkUpdate {
  work_id: number;
  title?: string;
  author?: string;
  summary?: string;
  my_comment?: string;
  published?: DateEdit | null;
  completed?: DateEdit | null;
  /** `has_file` | `record_only` */
  status?: string;
  /** A word count typed by the user, for files that carry no count. */
  word_count?: number;
  /** ISO 639-1 code, for files that carry no language metadata. */
  language?: string;
  /** Display form of the language, shown next to the code. */
  language_label?: string;
  tags?: string[];
}
