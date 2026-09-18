-- Where a project's code lives, typed by host: the `Repository` enum as
-- JSON, null until a hook binds the project and brings its remote. The
-- host is what an integration keys on; code evidence anchors never
-- name it — a commit sha is the same in every clone.
alter table projects add column repository jsonb;
