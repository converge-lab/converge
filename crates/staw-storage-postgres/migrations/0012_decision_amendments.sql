-- Amendments: what was learned about a decision after it was recorded.
--
-- A decision is a record, so what it said stays what it said. When
-- something changes — a signal is confirmed, the code moves on, a gate
-- is dropped — the change is a dated note added beside it, signed by
-- whoever made it, and never rewritten or removed. Before this table
-- those notes were a convention inside `consequences` ("Amended
-- 2026-09-20: …"), which anyone could edit and nothing attributed.
--
-- The author is a user, an agent, or a user working through an agent —
-- the same three shapes as `decision_author`, stored the same way.
create table decision_amendments (
    id           uuid primary key,
    decision_id  uuid not null references decisions(id) on delete cascade,
    body         text not null check (length(btrim(body)) > 0),
    author_user  uuid references users(id) on delete set null,
    author_agent uuid references agents(id) on delete set null,
    captured_at  timestamptz not null default now(),
    check (author_user is not null or author_agent is not null)
);
create index on decision_amendments (decision_id, captured_at);
