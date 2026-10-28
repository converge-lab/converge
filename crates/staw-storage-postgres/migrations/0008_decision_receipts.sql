-- Who has been shown which decision, the same shape as `signal_receipts`:
-- one row per (decision, user, session), `session = ''` for the person
-- reading on the web. "New for you" is the absence of a row, in any
-- session, which is what makes it cross-device. The session's own row
-- lives in `signal_sessions` and is shared by both kinds.
create table decision_receipts (
    decision_id uuid not null references decisions(id) on delete cascade,
    user_id     uuid not null references users(id) on delete cascade,
    session     text not null,
    at          timestamptz not null default now(),
    primary key (decision_id, user_id, session)
);
create index on decision_receipts (user_id, session);
