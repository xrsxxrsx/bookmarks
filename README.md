# bookmarks

A local, offline replacement for an AO3 bookmarks page: keeps a personal library of
fanfiction on disk so a work that is hidden or deleted upstream is still findable.

Status: **runnable desktop application.** Import, grouping, deduplication, translation
pairing, search, filtering, sorting and editing all work end to end. Remaining work is
listed at the bottom.

## Running it

```
npm install
npm run app:dev          # dev server + window with hot reload
npm run app:build        # Windows .exe + installers (NSIS, MSI)
```

Settings live in `%APPDATA%\com.bookmarks.app\settings.json` — a few hundred bytes holding
the library location, theme, font size and grouping mode. The identifier determines that
path, so changing it makes an existing configuration invisible.

The library itself — `bookmarks.db` plus `library/` — is deliberately kept out of
`%APPDATA%`, so a nearly full C: drive cannot stop it working. Set it in the settings
panel; the default when nothing has been chosen is `D:\dsh\books`, and `BOOKMARKS_LIBRARY`
overrides both, which keeps tests and scratch runs off the real library. The status bar
shows the resolved path, and the toolbar opens the folder for backing it up.

Because `D:\dsh\books` is this machine's layout rather than a sensible default for anyone
else, changing `DEFAULT_LIBRARY` in `crates/app/src/lib.rs` is the first thing to do on a
new checkout.

## Layout

```
crates/core                       domain logic — no UI, no Tauri
  src/ao3.rs                        AO3 download parser
  src/import.rs                     two-phase import pipeline
  src/store.rs                      the library: numbered work directories, file copies
  src/view.rs                       read models the interface consumes
  src/text.rs                       folding / search matching / word counting
  src/date.rs                       dates with partial precision and estimates
  src/fileformat.rs                 format classification and encoding detection
  migrations/                       schema, embedded into the binary at compile time
crates/app                        desktop shell — window, IPC commands, library location
  src/lib.rs                        command surface and DTOs
  tauri.conf.json                   window, bundle and security configuration
  capabilities/default.json         what the window is allowed to do
src/                              interface (React + TypeScript + Vite)
  library.ts                        filter / sort / group / search, all in memory
  api.ts                            the only place that talks to Rust
scripts/                          environment bootstrapping and icon generation
```

`cargo test` runs only `crates/core` by default (see `default-members`), keeping the fast
suite fast. Use `--workspace` to include the app crate.

## What is verified

160 Rust tests, plus a type-check of the interface (`npm run typecheck`). The frontend has
no unit tests of its own; see *Remaining work* below for why that matters.

Part of the suite is an acceptance test for the parser and the import pipeline. It runs
against three genuine AO3 downloads rather than hand-written fixtures, so the contract is
checked against what AO3 actually emits:

| file | work | role |
|---|---|---|
| `One_two_three.html` | works/7929115, English, 67,851 words | original |
| `Er_San.html` | works/14885858, Chinese, 94,736 words | translation of the above |
| `Zong_Lu_Jian_Dan_Di_Sha.html` | works/91520771, Chinese, 22,210 words | unrelated |

Those downloads are **not committed** — they are other people's stories, and a public
repository would republish them. On this machine they sit in `D:\dsh\books\_original-downloads`;
copy them into the repository root to run those 21 tests. Without them each test skips and
the suite stays green. Skips are printed, so they are visible under
`cargo test -- --nocapture`:

```
SKIP real_samples: sample file(s) not present: Er_San.html
test result: ok. 17 passed; 0 failed; 0 ignored
```

Note that a skip is reported as `passed`, not `ignored`: Rust has no runtime "ignored"
state, so the guards return early through a `Result` signature instead.

The behaviours that carry the design:

- importing the original and the translation **pairs them with no manual step**
- importing either one alone records the other as pending, and the pair completes by
  itself when the second file arrives — possibly in a later session
- re-importing a folder is a **no-op**: content hashes catch the duplicates and no
  metadata is rewritten, so a manual correction survives
- a work and its translation stay separate records, joined by `work_relations`
- the original's own German translation, which is not in the library, is reported as a
  suggestion rather than lost

## The interface

A table of works with a detail panel for editing. Everything is in memory, so filtering and
sorting are instant at this scale.

- **Counting** reports works and files separately: `共 87 篇 · 84 篇有文件 · 3 篇仅记录 · 126 个文件`.
  Filtering adds `筛选出 12 / 87 篇`, so it is always clear which number is which.
- **Search** covers title, author, tags, AO3 tags, your comment and the summary, with
  optional field prefixes (`tag:慢热`, `title:杀人`, `author:lisabart`). Punctuation and
  case are folded, so `一二三` finds `一，二，三`.
- **Sorting** by publication date, word count, insertion order or title, ascending or
  descending. Works with no date sort last either way, because "unknown" is not "very old".
- **Grouping** keeps a work and its translations adjacent; the toggle turns it off for a
  strictly flat list, which is what makes "sort by word count" trustworthy. The original
  always leads its group whatever the sort says — anchoring on rank alone would put a newer
  translation above the work it translates whenever sorting by date descending.
- **Tags** in a row are clickable: clicking one filters to works carrying it, and clicking
  it again clears the filter. The tag list in the toolbar is derived from the works rather
  than from a snapshot taken at load time, so a tag added in the edit panel appears at once.
- **Editing** is one block rather than a heading plus a separate form: title, author, status,
  publication and completion dates, tags, summary and your comment are all fields. Dates
  accept `2019`, `2019-06` or `2019-06-09` and record the precision actually given, with an
  `约` checkbox for a value that was guessed. Both dates are optional — an empty completion
  date just means unfinished, abandoned, or unknown.
- **Columns** are resizable by dragging a header edge, and there are four **font sizes** in
  the toolbar. Every size in the stylesheet is in `rem`, so the font setting scales the whole
  interface at once rather than only some of it.
- **Provenance is visible**: estimated word counts read `~23,159` and approximate dates
  read `~2019`, so an AO3-stated number is never confused with a guess.

## Colour

The palette is blue, derived from `#0000ff`. That literal value is deliberately **not** used
for text: on the dark background it measures only **2.20:1** contrast, below even the 3.0
threshold for non-text UI, because pure blue is the darkest primary and the eye is least
sensitive to it — it reads as dim rather than vivid. The accent is the same hue lifted in
lightness:

| role | colour | contrast on `--bg` |
|---|---|---|
| accent (links, active state) | `#6b7cff` | 5.37:1 |
| emphasis (hover) | `#8fa0ff` | 7.79:1 |
| body text | `#e8ecf8` | 16.0:1 |
| muted text | `#9aa6c4` | 7.78:1 |

`node scripts/pick-palette.mjs` prints the whole comparison, including the reference figure
for the literal `#0000ff`. Only the warning chip stays warm (`#e8a05a`), so a problem stands
out against an otherwise cool interface.
- **Import shows a plan first** — new work, attach, replace, or skip-as-duplicate, per file
  — and the plan is rebuilt from the source paths on confirmation, so it cannot be applied
  against a library that changed underneath it.

## Where files are stored

The library is the large part — the database plus every imported file — so it lives outside
the application data directory:

```
%APPDATA%\com.bookmarks.app\
  settings.json          ← a few hundred bytes, and nothing else
D:\dsh\books\            ← the default library, changeable in Settings
  bookmarks.db
  library/
    000001/work.html
    000002/work.html
    000003/work.html
    000004/
      work.epub
      work.pdf
```

Settings must live outside the library because the library's own location is one of them: it
has to be readable *before* the library is opened. Keeping them in `%APPDATA%` also means a
nearly full C: drive does not affect the library, and backing up is "copy the library folder".

**Changing the location** is done in Settings. The move is copy → verify → swap → delete, so a
failure part-way leaves the library where it was rather than half-moved, and a destination that
already holds another library is refused outright instead of being overwritten.

Directories are numbered by work id and files are named `work.<ext>`, so several formats of one
work coexist without `(1)` suffixes, a title containing characters Windows forbids cannot break
the path, and re-importing a format replaces it in place. The original filename is kept in the
database for display.

### Several versions of one file

A serialised work gets updated, or a finished work is revised and reposted, so a work can
legitimately hold more than one file of the same format:

```
000001/
  work.html        ← current version
  work.v2.html     ← an earlier revision, kept
```

Exactly one file per (work, format) is `current`; the rest are `archived`. The import decides
which happens, and the default is to **keep both** — a re-import usually means the work was
updated, and a lost revision is worse than an extra copy. The default can be flipped in Settings,
or overridden for a single import from the preview. Identical content is never a new version.
Archived versions can be promoted back to current, and the current version cannot be deleted
without promoting another first, so a work cannot end up with no readable file.

## Design decisions worth knowing

**AO3 downloads contain no `<meta name="ao3:*">` tags.** Everything is read from the DOM.
A parser written against those non-existent tags fails silently, matching nothing while
reporting no error.

**A work is one language edition, not one story.** An original and its translation are two
rows joined by `work_relations`, because a translation has a different author, language,
word count and personal note. Formats of the *same* edition (html/epub/txt/pdf) are files
under one work.

**Import is two-phase.** `analyze` proposes, `execute` applies. Grouping is the step that
can be wrong in a way the user cares about, so it is inspectable and testable before
anything is written.

**Grouping is evidence-based, in order of strength.** AO3 work id first (it survives a
renamed file); then filename stem matching a stored title. A metadata-free file whose name
matches nothing becomes its **own** work rather than being attached on a guess — a silent
wrong merge corrupts an existing record, whereas an extra entry costs one click to merge.

**Word counts and dates carry their provenance.** AO3's stated numbers are stored exactly
(67,851 — not an estimate). The counter used for PDF/TXT runs high against AO3's own figure:
+4.3% on a Chinese work, +1.6% on an English one, so estimated values are labelled and shown
with `~`. Dates are stored as a normalised ISO string plus a precision (`day`/`month`/`year`)
plus an `is_approx` flag, so "the source only stated 2019" stays distinguishable from "the
user guessed 2019".

**Search folds punctuation instead of using an index.** A query for `一二三` must find
`一，二，三`, which SQLite's default tokenizer cannot do for Chinese (a run of Han characters
becomes one token) and the trigram tokenizer cannot do under three characters. Matching is
substring search over normalised text, implemented once in `src/text.rs` and mirrored in
`src/library.ts::fold`. Revisit above roughly 10k works.

**The whole library is read in one call.** Filtering, sorting, grouping and searching happen
in the interface, so those rules live in one file instead of being split between SQL and
TypeScript. At this scale it is also faster than a round-trip.

**Tags are unique case-insensitively**, and AO3's own tags are kept in a separate field with
an explicit "add to my tags" action, so the user's filtering vocabulary is not diluted.

**PDF contributes a file record and no word count.** Text extraction needs a dedicated
engine and fails on scanned files, so no number is invented. A count can be typed in by
hand and is then stored with `word_count_source = 'manual'`, which is also what protects it
from being overwritten on a later import.

**Opening a file checks it is inside the library first.** The path comes from the database,
but a tampered row must not be able to hand an arbitrary path to the shell.

## Verifying the interface

`scripts/verify-ui.mjs` drives the running window over the Chrome DevTools Protocol and
asserts on the rendered result — 22 checks covering title alignment, slot widths, font
scaling, column resizing, tag filtering, the edit form and the palette. It exists because
launching the app and seeing that the process is alive proves nothing about what the window
shows, and several real bugs (a translation leading its own group, titles shifted out of
line by a badge, a toolbar that reflowed to two rows) were caught by these assertions rather
than by inspection.

```
npm run dev:win                  # starts the dev server and the window, debug port included
node scripts/verify-ui.mjs 9223
```

`npm run dev:win` enables the debugging port itself (see `scripts/dev.mjs`), so there is
nothing to set by hand.

It drives the **real** window against the **real** library, so the tag it adds to prove the
filter updates is a genuine write; it removes that tag again at the end, so the library is
left as it was found.

`scripts/inspect-ui.mjs` prints the rendered interface instead of asserting on it, which is
useful when something looks wrong and the question is what the DOM actually contains.

## Network access in this environment

The sandbox's restricted token breaks Schannel, so TLS from cargo, curl, PowerShell and git
fails (`SEC_E_NO_CREDENTIALS`), while Node's own TLS stack reaches the same hosts normally.
Two scripts work around this; **neither is needed on a normal machine**:

- `scripts/fetch-rustup.mjs` downloads the Rust toolchain using Node's TLS, because
  `rustup-init.exe`'s own downloader cannot complete a handshake here.
- `scripts/net-bridge.mjs` is a local HTTP→HTTPS bridge that lets cargo fetch crates. Point
  cargo at it via `.cargo/config.toml`, then start it:

  ```
  node scripts/net-bridge.mjs 23456 &
  cargo build --workspace
  ```

  On a machine with working TLS, delete `.cargo/config.toml` (or its two registry lines)
  and cargo uses the public HTTPS registry directly. `npm install` works through the system
  proxy without help. The file is git-ignored, since it points at `127.0.0.1` and would
  break `cargo build` anywhere else.

`scripts/make-icons.mjs` generates the application icons, writing PNGs and an ICO with a
small built-in encoder so no image tooling is required.

## Remaining work

1. **CSV import** from the AO3 bookmarks bookmarklet. Works with no file can already be
   entered by hand (`status = 'record_only'`), and they are counted separately from works
   that have files.
2. **Manual "link to…" action**, the fallback for pairing a translation whose file carries no
   AO3 id. Automatic pairing only sees what AO3's markup states.
3. **EPUB text extraction**, so EPUB files contribute a word count and searchable text.
   Today an EPUB is stored and listed, but its count has to be typed in.
4. **PDF text extraction** is deliberately not attempted. The sample that prompted it is
   9.5 MB for a 57 KB story, which means scanned page images rather than text; extraction
   would need OCR to be worth anything.
5. **Frontend unit tests.** Filter, sort, group and search logic live in `src/library.ts` and
   are type-checked but not covered by tests; folding parity with the Rust side is the most
   valuable thing to pin down.
6. **Packaging**: `npm run app:build` produces the installers. Code signing is skipped — an
   unsigned `.exe` triggers SmartScreen on first run.
7. **WAL checkpoint on exit**, so a backup of the library needs only `bookmarks.db` rather
   than the `-wal` and `-shm` files as well. The `-wal` file currently holds most of the
   recent writes, which makes "copy the database" subtly unsafe.

