-- Membership: the ACL. Every group gets exactly one owner (a column —
-- structural, not a role row) and flat members; visibility is
-- owner-or-member and everything inside a group inherits it.
--
-- owner_id is nullable ONLY as a migration window: pre-ACL rows have no
-- owner until the boot adoption pass (`Memberships::adopt`) hands them
-- to the deployment user. Every application write sets it.

alter table groups
    add column owner_id uuid references users(id);

create index groups_owner on groups(owner_id);

create table memberships (
    group_id   uuid not null references groups(id) on delete cascade,
    user_id    uuid not null references users(id) on delete cascade,
    invited_by uuid not null references users(id),
    created_at timestamptz not null default now(),
    primary key (group_id, user_id)
);

-- The visibility predicate probes by user; the PK serves group-side reads.
create index memberships_user on memberships(user_id);

-- THE visibility predicate — one definition of the ACL, used by every
-- scoped query (null viewer never reaches it; queries guard with
-- `$v is null or …`). A plain single-SELECT `stable` SQL function so the
-- planner inlines it into the calling query.
create function group_visible(gid uuid, viewer uuid) returns boolean
language sql stable as $$
    select exists (
        select 1 from groups g
        where g.id = gid
          and (g.owner_id = viewer
               or exists (select 1 from memberships m
                          where m.group_id = gid and m.user_id = viewer))
    )
$$;

-- Two people are mutually visible when some group holds both (as owner
-- or member) — the user-list scope.
create function shares_group(a uuid, b uuid) returns boolean
language sql stable as $$
    select exists (
        select 1 from groups g
        where (g.owner_id = a
               or exists (select 1 from memberships m
                          where m.group_id = g.id and m.user_id = a))
          and (g.owner_id = b
               or exists (select 1 from memberships m
                          where m.group_id = g.id and m.user_id = b))
    )
$$;
