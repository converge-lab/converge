# Converge

Decision memory for teams and their agents.

Converge captures the decisions a team makes — ADR-style records with
context, alternatives, and consequences — where the work happens, and keeps
them as a living graph: decisions supersede earlier ones, cross-reference
each other, and stay comparable long after the conversation that produced
them is gone. Humans and AI agents read and write the same memory: a web UI
for people, an [MCP](https://modelcontextprotocol.io) endpoint for agents,
and one REST API under them both.

## Quickstart

```sh
docker compose up --build
docker compose exec converge converge-server token mint   # prints your bearer token
```

Authentication is always on; the mint command prints a `cvg_…` secret to
your terminal (never to logs, where collectors would keep it). Then:

- **Web UI**: <http://127.0.0.1:8080>
- **MCP** (Claude Code shown; any MCP client works):

  ```sh
  claude mcp add --transport http converge http://127.0.0.1:8080/mcp \
    --header "Authorization: Bearer cvg_..."
  ```

  Agents get four tools: `project_list` (discover ids), `decision_add`
  (record an ADR, with creation-time supersession), `decision_get` (the
  full record plus graph edges), and `decision_list` (filter by
  project/group/status — `superseded` matches the *derived* status).

- **REST**: `GET`/`POST` `/api/v1/{groups,projects,decisions,users,agents}`
  with cursor pagination (`?limit=&cursor=` → `{items, next_cursor}`),
  atomic edit batches via `PATCH`, graph reads at `/decisions/{id}/edges`,
  and read-only relation projections (`/groups/{id}/decisions` is the
  group-wide feed).

> **Security note:** every surface except the health probe and the static
> assets requires a bearer token; OAuth (GitHub) and browser sessions are
> the next milestone. The compose file publishes the port on the host
> loopback only — a sane default until you put TLS in front.

## Development

Rust 1.88+. Docker is needed for the dev database and integration tests,
never for building (queries compile against the committed `.sqlx/` cache).

```sh
cargo check                 # no database needed — queries check against .sqlx/
cargo test                  # integration tests boot throwaway Postgres containers

cargo xtask db              # dev Postgres: boots, migrates, prints DATABASE_URL
cargo xtask prepare         # regenerate .sqlx/ after changing queries or schema
cargo xtask prepare --check # verify .sqlx/ is current (CI)

# the web app, live-reloading against a locally running converge-server:
rustup target add wasm32-unknown-unknown && cargo install trunk
cd crates/converge-web && trunk serve --features api   # http://127.0.0.1:8081
```

Configuration is layered per key — `/etc/converge/config.toml` ←
`$XDG_CONFIG_HOME/converge/config.toml` ← `./converge.toml` ←
`$CONVERGE_CONFIG` ← `CONVERGE_*` env (nested keys use `__`, e.g.
`CONVERGE_LOG__FILTER`, `CONVERGE_WEB__ASSETS`). The only required setting
is `database_url`.

## Workspace

| Crate | What it is |
|---|---|
| `converge-storage` | The domain model (groups → projects → decisions, the decision graph, users/agents) and the per-resource `Storage` traits the product is written against. |
| `converge-storage-postgres` | The PostgreSQL backend — compile-time-checked queries over the schema in `migrations/`. |
| `converge-server` | The product binary: REST under `/api/v1`, the stateless `/mcp` endpoint, and same-origin serving of the web bundle. |
| `converge-client` | Typed HTTP client over the same domain types — compiles natively (the future CLI) and to wasm (the web UI). |
| `converge-ui` | The component library (Leptos): design tokens, atoms, molecules. |
| `converge-web` | The web app (Leptos CSR → wasm), composed from `converge-ui`, talking to the API through `converge-client`. |
| `xtask` | Dev tooling (`cargo xtask …`): a throwaway dev Postgres and sqlx offline-cache generation. |

## Design, in one paragraph

Everything above the storage seam is backend-agnostic: the server, client,
and web app are written against the per-resource `Storage` traits and a
shared wire contract (the domain types *are* the wire format), so a
contract change is a compile error, not a runtime surprise. Ids are ULIDs
(stored as native `uuid`; `order by id` is capture order), `superseded` is
always derived from supersession edges rather than stored, instants are
server-assigned RFC3339 UTC only, edits apply as atomic batches of sparse
operations, and every surface that creates users or agents resolves them by
natural key (`ensure`), never scan-then-create.

## License

[MIT](LICENSE). Vendored fonts (Geist, Geist Mono) are under the SIL Open
Font License — see `crates/converge-ui/style/fonts/OFL.txt`.
