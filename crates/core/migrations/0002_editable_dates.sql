-- 0002_editable_dates.sql — make both dates fully editable.
--
-- `published_at` already carried a precision and an estimate flag, but `completed_at` did
-- not, so a completion date the user only knows to the year had nowhere to record that.
-- Without it the interface would have to either invent a day or refuse the edit.
--
-- Added as separate columns rather than by rebuilding the table: SQLite supports
-- `ALTER TABLE ... ADD COLUMN`, the new columns are nullable with defaults, and rebuilding
-- `works` would mean copying every row and every dependent foreign key for no benefit.

ALTER TABLE works ADD COLUMN completed_prec TEXT;
ALTER TABLE works ADD COLUMN completed_is_approx INTEGER NOT NULL DEFAULT 0;
ALTER TABLE works ADD COLUMN completed_source TEXT;  -- 'ao3' | 'csv' | 'manual'
