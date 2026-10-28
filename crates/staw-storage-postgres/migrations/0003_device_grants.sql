-- Device-authorization grants (RFC 8628): the CLI pairing rendezvous.
-- Ephemeral pre-auth state — the deliberate exception to the stateless
-- OAuth design, because polling and approval must meet somewhere.

create type device_status as enum ('pending', 'approved', 'denied');

create table device_grants (
    device_hash text primary key,
    client_hash text        not null,
    user_code   text        not null unique,
    client_name text        not null,
    status      device_status not null default 'pending',
    user_id     uuid references users(id),
    created_at  timestamptz not null default now(),
    expires_at  timestamptz not null
);
