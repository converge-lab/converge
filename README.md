# Converge

Decision memory for teams and their agents.

Converge captures the decisions a team makes — ADR-style records with
context, alternatives, and consequences — where the work happens, and keeps
them as a living graph: decisions supersede earlier ones, cross-reference
each other, and stay searchable long after the conversation that produced
them is gone. Humans and AI agents read and write the same memory.

## Status

Early days — the project is being built storage-first, one vertical slice
at a time. What exists today is the domain model and the PostgreSQL-backed
storage layer; the server (MCP + HTTP), CLI, and web UI follow.

## Workspace

| Crate | What it is |
|---|---|
| `converge-storage` | The domain model (decisions, the decision graph) and the `Storage` trait the product is written against. |
| `converge-storage-postgres` | The PostgreSQL backend — compile-time-checked queries over the schema in `migrations/`. |
| `xtask` | Dev tooling (`cargo xtask …`): a throwaway dev Postgres and sqlx offline-cache generation. |

## Development

Rust 1.88+; Docker is needed only for the dev database and integration
tests, never for building.

```sh
cargo check                 # no database needed — queries check against .sqlx/
cargo test                  # integration tests boot throwaway Postgres containers

cargo xtask db              # dev Postgres: boots, migrates, prints DATABASE_URL
cargo xtask prepare         # regenerate .sqlx/ after changing queries or schema
cargo xtask prepare --check # verify .sqlx/ is current (CI)
```

The storage model in one paragraph: ids are ULIDs (stored as native `uuid`,
so `order by id` is chronological); a decision's `superseded` status is
derived from inbound supersedes-edges rather than stored, so the graph and
the status can't drift apart; edits are applied as an atomic batch of
sparse operations.

## License

[MIT](LICENSE)
