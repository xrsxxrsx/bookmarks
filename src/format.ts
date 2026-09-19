/**
 * Small helpers shared by the components.
 */
import type { FileEntry } from './types';

/** `1,234` — group digits so a word count can be read at a glance. */
export function groupDigits(n: number): string {
  return n.toLocaleString('en-US');
}

/** Human-readable file size. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * How a file's format is labelled: the coarse class, plus the real extension when the file
 * landed in the catch-all `other` class, e.g. `Other · EPUB`.
 */
export function formatLabel(file: FileEntry): string {
  const cls = file.format_class;
  const detail = file.format_detail;
  if (cls === 'other') {
    return `Other · ${detail.toUpperCase()}`;
  }
  return detail.toUpperCase();
}

/** A short badge for a language, e.g. `EN`, `ZH`. */
export function languageBadge(code: string | null): string {
  if (code === null || code === '') return '?';
  return code.toUpperCase();
}

/**
 * Returns the parts of `text` that match `query`, for highlighting.
 *
 * Punctuation-insensitive matching means the offsets in the folded text do not map back to
 * the original string. Rather than fake precision, a field that matches is highlighted as
 * a whole when the query cannot be located literally. Getting this wrong would highlight
 * the wrong characters, which is worse than not highlighting at all.
 */
export function highlight(text: string, query: string): { text: string; hit: boolean }[] {
  const trimmed = query.trim();
  if (trimmed === '') return [{ text, hit: false }];

  // Only literal, case-insensitive matches are highlighted.
  const needle = trimmed.toLowerCase();
  const haystack = text.toLowerCase();
  const parts: { text: string; hit: boolean }[] = [];
  let index = 0;

  for (;;) {
    const found = haystack.indexOf(needle, index);
    if (found === -1) break;
    if (found > index) parts.push({ text: text.slice(index, found), hit: false });
    parts.push({ text: text.slice(found, found + needle.length), hit: true });
    index = found + needle.length;
  }
  if (index < text.length) parts.push({ text: text.slice(index), hit: false });
  return parts.length > 0 ? parts : [{ text, hit: false }];
}
