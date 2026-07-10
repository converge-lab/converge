-- Decision core: groups, projects, authors, decisions, and the
-- decision→decision graph edges (supersession + cross-refs).
--
-- Ids are ULIDs stored as native `uuid` — ULID and UUID are the same 128-bit
-- value, differing only in text encoding. Rust converts at the boundary
-- (the `ulid` crate's `uuid` feature); the timestamp-first ULID layout keeps
-- `uuid` byte-order = chronological, so `order by id` is time-ordered.

create type group_kind      as enum ('shared', 'personal');
create type agent_kind      as enum ('model', 'tool');
create type decision_status as enum ('accepted', 'draft', 'proposed', 'superseded', 'rejected');

-- A group/team; owns projects.
create table groups (
    id          uuid primary key,
    name        text not null,
    description text,
    kind        group_kind not null,
    created_at  timestamptz not null default now()
);

-- A logical project (codebase/service), owned by a group.
create table projects (
    id          uuid primary key,
    group_id    uuid not null references groups(id) on delete cascade,
    name        text not null,                                     -- display only
    description text,
    created_at  timestamptz not null default now()
);
create index on projects (group_id);

-- The two author kinds. Both resolve by natural key: `ensure` is a
-- deterministic create-if-absent (insert … on conflict), never
-- scan-then-create.
create table users (
    id       uuid primary key,
    provider text not null,                                         -- asserting auth provider
    subject  text not null,                                         -- provider's immutable id
    handle   text not null,                                         -- login; refreshed on login
    name     text not null,                                         -- display; refreshed on login
    unique (provider, subject)                                      -- identity
);
create table agents (
    id   uuid primary key,
    kind agent_kind not null,
    name text not null,
    unique (kind, name)                                             -- natural key
);

-- Bearer tokens, hashed at rest — the secret is generated and shown once
-- by the server, never stored. The bootstrap admin token is minted on
-- first boot.
create table tokens (
    id         uuid primary key,
    user_id    uuid not null references users(id) on delete cascade,
    hash       text not null unique,
    label      text not null,
    created_at timestamptz not null default now()
);
create index on tokens (user_id);

-- The ADR record AND the graph node.
create table decisions (
    id           uuid primary key,
    project_id   uuid not null references projects(id) on delete cascade,
    status       decision_status not null,
    title        text not null,
    summary      text not null default '',
    context      text,
    consequences text,
    alternatives jsonb not null default '[]',                      -- [{option, why_rejected}], ordered
    captured_at  timestamptz not null default now()
);
create index on decisions (project_id);
create index on decisions (project_id, status);

-- Authorship: many per decision; (user?, agent?), at least one present.
create table decision_author (
    decision_id uuid not null references decisions(id) on delete cascade,
    user_id     uuid references users(id),
    agent_id    uuid references agents(id),
    check (user_id is not null or agent_id is not null),
    unique nulls not distinct (decision_id, user_id, agent_id)     -- pg 15+
);
create index on decision_author (decision_id);

-- Graph edges (decision → decision).
--
-- An inbound supersedes-edge makes the target read as `superseded`: that
-- status is derived at query time, never stored in decisions.status.
create table decision_supersedes (
    decision_id   uuid not null references decisions(id) on delete cascade,
    supersedes_id uuid not null references decisions(id) on delete cascade,
    check (decision_id <> supersedes_id),
    primary key (decision_id, supersedes_id)
);
create index on decision_supersedes (supersedes_id);          -- inbound + derived status

create table decision_related (
    decision_id uuid not null references decisions(id) on delete cascade,
    ref_id      uuid not null references decisions(id) on delete cascade,
    why         text,
    check (decision_id <> ref_id),
    primary key (decision_id, ref_id)
);
create index on decision_related (ref_id);                    -- inbound cross-refs

-- ─── Evidence: sessions + append-only message streams ──────────────────────

create type session_kind as enum ('transcript', 'slack', 'pr', 'incident');

-- A conversation container decisions cite: an agent transcript, a Slack
-- thread, a PR discussion, an incident channel. (kind, external) is the
-- natural key importers and live agents converge on.
create table sessions (
    id          uuid primary key,
    project_id  uuid not null references projects(id),
    kind        session_kind not null,
    external    text not null,                                      -- source system's reference
    title       text not null,                                      -- refreshed on ensure
    captured_at timestamptz not null default now(),
    unique (kind, external)
);
create index on sessions (project_id);

-- Append-only: the stream is evidence — no update path exists. seq carries
-- the conversation order (assigned under the session's row lock).
create table messages (
    id          uuid primary key,
    session_id  uuid not null references sessions(id) on delete cascade,
    seq         integer not null,
    speaker     text not null,                                      -- display string, not a user
    body        text not null,
    sent_at     timestamptz,                                        -- external fact (importers)
    captured_at timestamptz not null default now(),
    unique (session_id, seq)
);

-- Decision → message anchors, a set. The message FK has no cascade: an
-- evidenced message (and its session, transitively) is undeletable.
create table evidence (
    decision_id uuid not null references decisions(id) on delete cascade,
    message_id  uuid not null references messages(id),
    primary key (decision_id, message_id)
);
create index on evidence (message_id);
