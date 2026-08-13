-- A project delete takes its sessions (and their messages) with it —
-- the transcript record has no life of its own once the project is
-- gone. The evidence→message FK deliberately keeps NO cascade: a
-- message anchored as evidence by a *surviving* decision (another
-- project's) still blocks the delete, preserving the "an evidenced
-- message is undeletable" invariant; in-project anchors die with their
-- decisions before the messages go.
alter table sessions
    drop constraint sessions_project_id_fkey,
    add constraint sessions_project_id_fkey
        foreign key (project_id) references projects(id) on delete cascade;
