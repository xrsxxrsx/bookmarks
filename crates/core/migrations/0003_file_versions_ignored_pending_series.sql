-- 0003_file_versions_ignored_pending_series.sql
--
-- Three model additions, applied together so the schema changes once rather than three
-- times.
--
-- 1. Several versions of one file.
--    A serialised work gets updated, or a finished work is revised and reposted, so a work
--    can legitimately have more than one file of the same format. Until now one format was
--    one slot and a re-import replaced it, which loses the earlier revision. Files now carry
--    a role: exactly one `current` per (work, format), any number of `archived` ones.
--
-- 2. Ignoring a pending translation.
--    AO3 states a work's translations by id. A translation the user does not want (another
--    language, a translation they dislike) would otherwise be reported as a suggestion on
--    every launch. `dismissed` records the decision instead of deleting the row — deleting
--    it would only bring the suggestion back on the next import of the original.
--
-- 3. Series.
--    A work can belong to a series and also be a translation, so series is kept apart from
--    `work_relations`: a relation pairs two works, a series just tags membership in order.

-- Default 1 means existing rows become the current version, which is correct: before this
-- migration a (work, format) pair could only ever hold one file.
ALTER TABLE files ADD COLUMN role TEXT NOT NULL DEFAULT 'current';
ALTER TABLE files ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
-- When the version was superseded, so the interface can say how old an archived copy is.
ALTER TABLE files ADD COLUMN archived_at TEXT;

CREATE INDEX idx_files_work_format_role ON files(work_id, format_detail, role);

ALTER TABLE pending_relations ADD COLUMN dismissed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE pending_relations ADD COLUMN dismissed_at TEXT;

CREATE TABLE series (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL,
    note       TEXT,
    created_at TEXT NOT NULL
);

-- Series names are compared without case, like tags, so "Big Bang" and "big bang" do not
-- become two series.
CREATE UNIQUE INDEX idx_series_name ON series(name COLLATE NOCASE);

CREATE TABLE works_series (
    work_id   INTEGER NOT NULL REFERENCES works(id)  ON DELETE CASCADE,
    series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
    -- Position within the series. Nullable because membership is often known before the
    -- order is; the interface sorts unknown positions last rather than inventing a number.
    position  INTEGER,
    PRIMARY KEY (work_id, series_id)
);

CREATE INDEX idx_works_series_series ON works_series(series_id);
