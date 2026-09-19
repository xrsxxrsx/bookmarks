-- 0001_init.sql — initial schema for the local fanfiction library.
--
-- Design notes (see project plan for the full rationale):
--  * `works` represents ONE LANGUAGE EDITION of a story, not "a story".
--    A work and its translation are therefore two rows joined by `work_relations`,
--    not one row with two files. This keeps `language`, `author` and `my_comment`
--    unambiguous, because a translation has a different author and its own notes.
--  * `files` represents physical files. Several files (html/epub/txt/pdf) of the same
--    edition hang off one work. `format_class` is the coarse 3-way split the UI needs
--    (pdf | html | other); `format_detail` keeps the real extension so 'other' can be
--    displayed as "Other · EPUB" and reclassified later without a schema change.
--  * Word counts and dates both carry a provenance column, because imported values are
--    not equally trustworthy: AO3 states exact numbers, while PDF/TXT require estimation.

CREATE TABLE works (
    id                INTEGER PRIMARY KEY,

    title             TEXT    NOT NULL,
    author            TEXT,
    summary           TEXT,
    my_comment        TEXT,             -- AO3 bookmark notes / personal annotation

    -- Publication date, normalised to ISO-8601. For partial or estimated dates the
    -- missing components are filled with the start of their range (year -> -01-01)
    -- so sorting always has a definite value; `published_prec` decides how much of
    -- it is shown, and `date_is_approx` records that the value was guessed.
    published_at      TEXT,
    published_prec    TEXT,             -- 'day' | 'month' | 'year' | NULL
    date_is_approx    INTEGER NOT NULL DEFAULT 0,
    date_source       TEXT,             -- 'ao3' | 'csv' | 'epub' | 'filename' | 'manual'
    completed_at      TEXT,             -- AO3 "Completed:" date, when present

    word_count        INTEGER NOT NULL DEFAULT 0,
    word_count_source TEXT,             -- 'ao3' | 'estimated' | 'manual'

    language          TEXT,             -- ISO 639-1, e.g. 'en' / 'zh'
    language_label    TEXT,             -- display form, e.g. '中文-普通话 國語'

    translation_role  TEXT,             -- 'original' | 'translation' | NULL

    source_url        TEXT,
    external_id       TEXT,             -- AO3 work id: grouping + relation-matching key
    ao3_tags          TEXT,             -- JSON array; kept apart from the user's own tags

    status            TEXT NOT NULL DEFAULT 'has_file',  -- 'has_file' | 'record_only'

    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,

    CHECK (translation_role IS NULL OR translation_role IN ('original', 'translation')),
    CHECK (status IN ('has_file', 'record_only')),
    CHECK (date_is_approx IN (0, 1)),
    CHECK (published_prec IS NULL OR published_prec IN ('day', 'month', 'year'))
);

-- One AO3 work must not be imported twice.
CREATE UNIQUE INDEX idx_works_external_id
    ON works(external_id) WHERE external_id IS NOT NULL;

CREATE TABLE files (
    id            INTEGER PRIMARY KEY,
    work_id       INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,

    format_class  TEXT NOT NULL,        -- 'pdf' | 'html' | 'other'
    format_detail TEXT,                 -- 'pdf' | 'html' | 'epub' | 'txt' | ...

    original_name TEXT NOT NULL,        -- name as it was when imported
    rel_path      TEXT NOT NULL UNIQUE, -- path under the library root
    file_hash     TEXT,                 -- content hash, for duplicate detection
    size_bytes    INTEGER,

    word_count    INTEGER,              -- per-file count; NULL when it can't be known
    is_estimated  INTEGER NOT NULL DEFAULT 0,

    imported_at   TEXT NOT NULL,

    CHECK (format_class IN ('pdf', 'html', 'other')),
    CHECK (is_estimated IN (0, 1))
);

CREATE INDEX idx_files_work_id ON files(work_id);

CREATE TABLE tags (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL
);

-- NOCASE keeps "Fluff" and "fluff" from becoming two different tags.
CREATE UNIQUE INDEX idx_tags_name ON tags(name COLLATE NOCASE);

CREATE TABLE work_tags (
    work_id INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    tag_id  INTEGER NOT NULL REFERENCES tags(id)  ON DELETE CASCADE,
    PRIMARY KEY (work_id, tag_id)
);

CREATE INDEX idx_work_tags_tag_id ON work_tags(tag_id);

-- Relations between separate works (a translation, a remix, ...).
-- Symmetric: the pair is stored once with the smaller id first, so a single lookup
-- finds the partner from either side and "linked but reverse lookup misses it" is
-- impossible by construction.
CREATE TABLE work_relations (
    work_a INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    work_b INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    kind   TEXT NOT NULL DEFAULT 'translation',  -- 'translation' | 'remix' | 'related'
    source TEXT NOT NULL DEFAULT 'ao3_html',     -- 'ao3_html' | 'manual'
    PRIMARY KEY (work_a, work_b),
    CHECK (work_a < work_b),
    CHECK (kind IN ('translation', 'remix', 'related')),
    CHECK (source IN ('ao3_html', 'manual'))
);

-- A relation pointing at a work that has not been imported yet.
-- The original's HTML lists its translations by id before those files exist locally;
-- recording them here lets the pairing complete automatically at import time instead
-- of relying on the user to notice and link them by hand.
CREATE TABLE pending_relations (
    external_id  TEXT NOT NULL,        -- AO3 work id of the not-yet-imported partner
    kind         TEXT NOT NULL DEFAULT 'translation',
    language     TEXT,
    title        TEXT,
    author       TEXT,
    from_work_id INTEGER REFERENCES works(id) ON DELETE CASCADE,
    PRIMARY KEY (external_id, from_work_id)
);

-- NOTE: `schema_migrations` is deliberately not created here. The migration runner
-- creates it before applying any migration, so defining it in both places would collide.
