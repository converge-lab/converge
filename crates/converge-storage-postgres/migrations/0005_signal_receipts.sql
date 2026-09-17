-- Who has been shown which signal, and through what. Two questions live
-- here. A harness session asks "what has this model context not been
-- handed" — per (user, session), because two concurrent sessions of one
-- person each need a conflict in their own context. The web asks "has
-- this person seen it" — per (user, signal), because a person opens
-- signals out of order. One receipts table answers both; `session = ''`
-- is the person themself, reading on the web. The verdict (confirmed,
-- dismissed) stays on the signal: delivered and resolved are different.
create table signal_sessions (
    user_id    uuid not null references users(id) on delete cascade,
    session    text not null,
    -- claude | codex | opencode | cursor — the agent tool the session ran in.
    harness    text,
    -- Only signals recorded after this instant are handed to the session:
    -- what came before is the session-start listing's job, not a backlog.
    started_at timestamptz not null default now(),
    seen_at    timestamptz not null default now(),
    primary key (user_id, session)
);
create index on signal_sessions (seen_at);

create table signal_receipts (
    signal_id uuid not null references signals(id) on delete cascade,
    user_id   uuid not null references users(id) on delete cascade,
    session   text not null,
    at        timestamptz not null default now(),
    primary key (signal_id, user_id, session)
);
create index on signal_receipts (user_id, session);
