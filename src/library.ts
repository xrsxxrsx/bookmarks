/**
 * Filtering, sorting, grouping and search — all in memory.
 *
 * A personal library is small enough that this belongs in one place rather than being
 * split between SQL and the UI. Keeping it here also means the rules are directly
 * testable and change together.
 */
import type { Work } from './types';

export type SortField = 'published' | 'words' | 'added' | 'title';
export type SortDirection = 'asc' | 'desc';

/** How related works are kept together. Mirrors the stored setting. */
export type GroupMode = 'translation' | 'series' | 'none';

export interface ViewOptions {
  sortField: SortField;
  sortDirection: SortDirection;
  /** Which relation to group by, or none for a strictly flat list. */
  groupMode: GroupMode;
  search: string;
  /** A work must carry all of these tags to pass. */
  requiredTags: string[];
  /** A work must be in one of these languages, when non-empty. */
  languages: string[];
  includeRecordOnly: boolean;
}

export const defaultViewOptions: ViewOptions = {
  sortField: 'published',
  sortDirection: 'desc',
  groupMode: 'translation',
  search: '',
  requiredTags: [],
  languages: [],
  includeRecordOnly: true,
};

/** A row in the table. `depth` is 1 for a work shown under its pair partner. */
export interface Row {
  work: Work;
  depth: 0 | 1;
  /** How many works share this row's group, so the UI can label it. */
  groupSize: number;
}

/** The fields a search term may be scoped to, plus the default "everywhere". */
type Scope = 'all' | 'title' | 'author' | 'tag' | 'comment' | 'summary' | 'language';

interface Query {
  terms: { scope: Scope; text: string }[];
}

const SCOPE_PREFIXES: [string, Scope][] = [
  ['title:', 'title'],
  ['author:', 'author'],
  ['tag:', 'tag'],
  ['comment:', 'comment'],
  ['summary:', 'summary'],
  ['lang:', 'language'],
];

/**
 * Parses a query into scoped terms.
 *
 * Field prefixes are supported but optional — `tag:慢热 hannibal` means "tagged 慢热 and
 * mentioning hannibal anywhere". Text is lower-cased but otherwise left alone: the
 * backend already folded the searchable text, and the query goes through the same folding
 * rules in {@link fold} so punctuation differences do not matter.
 */
export function parseQuery(raw: string): Query {
  const terms: { scope: Scope; text: string }[] = [];
  for (const piece of raw.split(/\s+/)) {
    if (piece === '') continue;
    const lower = piece.toLowerCase();
    const match = SCOPE_PREFIXES.find(([prefix]) => lower.startsWith(prefix));
    if (match && piece.length > match[0].length) {
      terms.push({ scope: match[1], text: piece.slice(match[0].length) });
    } else {
      terms.push({ scope: 'all', text: piece });
    }
  }
  return { terms };
}

/** The fields each scope searches, by work property. */
function scopeText(work: Work, scope: Scope): string {
  switch (scope) {
    case 'title':
      return work.title;
    case 'author':
      return work.author ?? '';
    case 'tag':
      return [...work.tags, ...work.ao3_tags].join(' ');
    case 'comment':
      return work.my_comment ?? '';
    case 'summary':
      return work.summary ?? '';
    case 'language':
      return `${work.language ?? ''} ${work.language_label ?? ''}`;
    case 'all':
      return work.search_text;
  }
}

/**
 * Matches one term against one work.
 *
 * Both sides are folded the same way the backend folds stored text, so a query typed
 * without punctuation still matches a title written with it.
 */
function termMatches(work: Work, scope: Scope, rawTerm: string): boolean {
  const term = fold(rawTerm);
  if (term === '') return false;
  return fold(scopeText(work, scope)).includes(term);
}

/**
 * Mirrors the backend's folding: lower-case, and collapse runs of punctuation to a single
 * space. Han characters are kept as one contiguous run so that a query typed without
 * commas still matches a title written with them.
 */
export function fold(input: string): string {
  let out = '';
  let pendingSeparator = false;
  let prevWasHan = false;
  // NFKC first so full-width forms collapse, matching the Rust side.
  for (const ch of input.normalize('NFKC').toLowerCase()) {
    const isAlnum = /[\p{L}\p{N}]/u.test(ch);
    if (!isAlnum) {
      pendingSeparator = true;
      continue;
    }
    const isHan = /\p{Script=Han}/u.test(ch);
    if (out !== '' && (pendingSeparator || isHan !== prevWasHan)) {
      out += ' ';
    }
    out += ch;
    pendingSeparator = false;
    prevWasHan = isHan;
  }
  return out.trim();
}

/** True when the work passes every filter. */
export function workMatches(work: Work, options: ViewOptions): boolean {
  if (!options.includeRecordOnly && work.status === 'record_only') return false;

  // Every selected tag must be present.
  for (const tag of options.requiredTags) {
    const lowered = tag.toLowerCase();
    if (!work.tags.some((t) => t.toLowerCase() === lowered)) return false;
  }

  if (options.languages.length > 0) {
    const code = work.language ?? '';
    if (!options.languages.includes(code)) return false;
  }

  const { terms } = parseQuery(options.search);
  for (const term of terms) {
    if (!termMatches(work, term.scope, term.text)) return false;
  }
  return true;
}

/**
 * Relevance of a search hit, used to order results when searching.
 *
 * A title match outranks a tag match, which outranks a hit in the summary, so searching a
 * title term floats that work to the top instead of leaving it wherever the sort put it.
 */
export function relevance(work: Work, search: string): number {
  const { terms } = parseQuery(search);
  if (terms.length === 0) return 0;

  const weights: Partial<Record<Scope, number>> = {
    title: 100,
    author: 40,
    tag: 30,
    language: 20,
    comment: 15,
    summary: 10,
    all: 5,
  };

  let total = 0;
  for (const term of terms) {
    const folded = fold(term.text);
    if (folded === '') continue;
    const scopes: Scope[] = term.scope === 'all' ? ['title', 'author', 'tag', 'language', 'comment', 'summary'] : [term.scope];
    for (const scope of scopes) {
      const text = fold(scopeText(work, scope));
      const weight = weights[scope] ?? 1;
      if (text === folded) total += weight * 3;
      else if (text.startsWith(folded)) total += weight * 2;
      else if (text.includes(folded)) total += weight;
    }
  }
  return total;
}

/**
 * Comparable value for a work's date. Works with no date sort last regardless of
 * direction, because "unknown" is not "very old".
 */
function dateKey(work: Work): string | null {
  return work.published_at;
}

function compareBy(field: SortField): (a: Work, b: Work) => number {
  switch (field) {
    /**
     * Dates are stored as ISO strings, so string comparison is chronological. A partial
     * date is stored at the start of its range, which means an approximate `2019` sorts
     * before an exact `2019-06-09` rather than after it.
     */
    case 'published':
      return (a, b) => {
        const ka = dateKey(a);
        const kb = dateKey(b);
        if (ka === null && kb === null) return a.id - b.id;
        if (ka === null) return 1;
        if (kb === null) return -1;
        return ka.localeCompare(kb);
      };
    case 'words':
      return (a, b) => a.word_count - b.word_count;
    case 'added':
      return (a, b) => a.id - b.id;
    case 'title':
      return (a, b) => a.title.localeCompare(b.title, 'zh-Hans-CN');
  }
}

/** Applies the direction, then a stable tie-break so the order never wobbles. */
function makeComparator(options: ViewOptions): (a: Work, b: Work) => number {
  const base = compareBy(options.sortField);
  const sign = options.sortDirection === 'asc' ? 1 : -1;
  return (a, b) => {
    const primary = base(a, b) * sign;
    return primary !== 0 ? primary : a.id - b.id;
  };
}

/**
 * Filters, sorts and groups the library into display rows.
 *
 * Grouping keeps related works together rather than letting the sort separate them, and the group
 * is anchored at the position of its best-ranked member. Which relation is used is a setting,
 * because a work can be both a translation and part of a series, and showing both at once would
 * need two levels of indentation — which eats the table's width for a combination that is rarely
 * wanted at the same time.
 */
export function buildRows(works: Work[], options: ViewOptions): Row[] {
  const filtered = works.filter((w) => workMatches(w, options));
  const search = options.search.trim();
  const comparator = makeComparator(options);

  const ranked = [...filtered].sort((a, b) => {
    if (search !== '') {
      const diff = relevance(b, search) - relevance(a, search);
      if (diff !== 0) return diff;
    }
    return comparator(a, b);
  });

  if (options.groupMode === 'none') {
    return ranked.map((work) => ({ work, depth: 0 as const, groupSize: 1 }));
  }

  const byId = new Map(ranked.map((w) => [w.id, w]));
  const emitted = new Set<number>();
  const rows: Row[] = [];
  const rankOf = new Map(ranked.map((w, i) => [w.id, i]));

  /** Members of this work's group that are still visible and unclaimed. */
  const partnersOf = (work: Work): Work[] => {
    if (options.groupMode === 'translation') {
      return work.related
        .map((r) => byId.get(r.work_id))
        .filter((w): w is Work => w !== undefined && !emitted.has(w.id));
    }
    // Series: works sharing a series name. The first shared name wins, since a work in two
    // series cannot be shown under both without duplicating it.
    const names = work.series.map((s) => s.name);
    if (names.length === 0) return [];
    const primary = names[0];
    if (primary === undefined) return [];
    return ranked.filter(
      (w) => w.id !== work.id && !emitted.has(w.id) && w.series.some((s) => s.name === primary),
    );
  };

  for (const work of ranked) {
    if (emitted.has(work.id)) continue;

    const members = [work, ...partnersOf(work)];

    /*
     * Series have a real reading order, so their declared positions win over the sort. A
     * translation has no inherent order, so the original leads and the sort decides the rest.
     */
    if (options.groupMode === 'series') {
      const positionOf = (w: Work): number => {
        const name = work.series.map((s) => s.name)[0];
        const entry = w.series.find((s) => s.name === name);
        return entry?.position ?? Number.MAX_SAFE_INTEGER;
      };
      members.sort((a, b) => {
        const byPosition = positionOf(a) - positionOf(b);
        if (byPosition !== 0) return byPosition;
        return (rankOf.get(a.id) ?? 0) - (rankOf.get(b.id) ?? 0);
      });
    } else {
      // The original leads its group whatever the sort says. Anchoring purely on rank would let a
      // newer translation sit above the work it translates whenever sorting by date descending.
      members.sort((a, b) => {
        const roleRank = (w: Work) =>
          w.translation_role === 'original' ? 0 : w.translation_role === 'translation' ? 1 : 2;
        const byRole = roleRank(a) - roleRank(b);
        if (byRole !== 0) return byRole;
        return (rankOf.get(a.id) ?? 0) - (rankOf.get(b.id) ?? 0);
      });
    }

    members.forEach((member, index) => {
      emitted.add(member.id);
      rows.push({
        work: member,
        depth: index === 0 ? 0 : 1,
        groupSize: members.length,
      });
    });
  }
  return rows;
}

/** Languages present in the library, with how many works each has. */
export function languageFacets(works: Work[]): { code: string; label: string; count: number }[] {
  const counts = new Map<string, { label: string; count: number }>();
  for (const work of works) {
    const code = work.language ?? '';
    if (code === '') continue;
    const existing = counts.get(code);
    if (existing) existing.count += 1;
    else counts.set(code, { label: work.language_label ?? code, count: 1 });
  }
  return [...counts.entries()]
    .map(([code, { label, count }]) => ({ code, label, count }))
    .sort((a, b) => b.count - a.count || a.code.localeCompare(b.code));
}

/**
 * The user's tags, with how many works carry each, derived from the works themselves.
 *
 * Deliberately derived rather than read from a separate list the backend also returns: that
 * list is a snapshot from load time, so a tag added in the edit panel would not appear in
 * the filter until the next full reload. Counting here also means the numbers cannot
 * disagree with what filtering actually does.
 *
 * Matching is case-insensitive because the backend stores tags under a case-insensitive
 * unique index, so `Fluff` and `fluff` are one tag.
 */
export function tagFacets(works: Work[]): { name: string; count: number }[] {
  const counts = new Map<string, { name: string; count: number }>();
  for (const work of works) {
    // A work cannot carry the same tag twice, but guard anyway so the count stays right.
    const seen = new Set<string>();
    for (const tag of work.tags) {
      const key = tag.toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      const existing = counts.get(key);
      if (existing) existing.count += 1;
      else counts.set(key, { name: tag, count: 1 });
    }
  }
  return [...counts.values()].sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
}
