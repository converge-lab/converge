-- What the repository said about an anchor, the last time anyone asked.
--
-- The anchor itself never changes: commit, path, lines, excerpt and
-- digest are what was cited and stay that way. These two columns are
-- the answer to a different question — does the repository still agree?
-- The server fetches the blob at that commit, takes the same lines,
-- normalizes them the same way and compares the hash. A match stamps
-- `verified_at`; anything else writes what went wrong in `mismatch`,
-- and the two are mutually exclusive.
--
-- Both null means nobody has asked: no App, no token, or a project
-- whose repository is not a host we can read.
alter table decision_code_evidence
    add column verified_at timestamptz,
    add column mismatch    text;
