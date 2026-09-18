-- Code evidence: a decision cites a line range in one file at one commit
-- of its project's repository, with the cited lines as they were and
-- their hash. Written once and never rewritten — a commit sha is the
-- same in every clone, so the anchor stays reproducible when lines
-- drift; the excerpt keeps it readable without the repository. Message
-- evidence keeps its own table (`evidence`); the two are the two kinds.
create table decision_code_evidence (
    decision_id uuid not null references decisions(id) on delete cascade,
    commit      text not null,
    path        text not null,
    line_start  integer not null,
    line_end    integer not null,
    excerpt     text not null,
    digest      text not null,
    primary key (decision_id, commit, path, line_start, line_end)
);
