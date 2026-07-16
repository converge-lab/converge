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

(`token list` / `token revoke <id>` manage tokens from the host; `--user
<handle>` provisions teammates without an identity provider — each then
manages their own via `/api/v1/tokens`.)

Authentication is always on; the mint command prints a `cvg_…` secret to
your terminal (never to logs, where collectors would keep it). Then:

- **Web UI**: <http://127.0.0.1:8080> — paste the token at the sign-in
  screen; it's exchanged for an `HttpOnly` session cookie (the secret is
  never stored in the browser).
- **MCP** (Claude Code shown; any MCP client works):

  ```sh
  claude mcp add --transport http converge http://127.0.0.1:8080/mcp \
    --header "Authorization: Bearer cvg_..."
  ```

  OAuth-capable MCP clients (claude.ai connectors) need no header: the
  server is a full OAuth authorization server (discovery, dynamic client
  registration, PKCE) — add the URL, sign in when the browser opens, done.
  Connector grants appear in the settings UI as `connector:…` tokens;
  revoking one cuts that connector off. Set `[auth] session_secret` (and
  `public_url` behind a proxy) — registered clients are signed by that
  key, so an unset (per-boot random) key orphans them on restart.

  Agents get the full palette: decisions (`decision_add` with
  supersession + evidence anchors, `decision_get`, `decision_list`),
  evidence ingest (`session_ensure`, `message_add`), and project mapping
  (`project_match`/`project_bind`/`project_dismiss`).

- **Claude Code integration** (`converge-cli`): one command wires it all —

  ```sh
  curl -fsSL https://raw.githubusercontent.com/converge-lab/converge/main/install.sh | sh
  converge init    # credentials → hooks → MCP registration, once per machine
  ```

  (The binary lands in `~/.converge/bin` with a `~/.local/bin` symlink;
  releases are checksum-manifested and minisign-signed. Building from
  source works too: `cargo install --path crates/converge-cli`.)

  After that, opening any repository in Claude Code suggests a project
  binding in-session (the human answers in conversation; hooks write the
  committed `.converge` marker), session transcripts sync into the
  evidence layer automatically, and every session starts with the
  project's decision index injected. `converge project init` is the
  manual fallback for binding from the terminal.

- **REST**: `GET`/`POST` `/api/v1/{groups,projects,decisions,users,agents}`
  with cursor pagination (`?limit=&cursor=` → `{items, next_cursor}`),
  atomic edit batches via `PATCH`, graph reads at `/decisions/{id}/edges`,
  and read-only relation projections (`/groups/{id}/decisions` is the
  group-wide feed).

> **Security note:** every surface except the health probe, the sign-in
> endpoints, and the static assets requires a credential — a bearer token
> (agents, CLI) or the session cookie (browser). The compose file
> publishes the port on the host loopback only — a sane default until you
> put TLS in front. Set `CONVERGE_AUTH__SESSION_SECRET` to keep browser
> sessions across restarts.

### Identity-provider sign-in (optional)

The auth core needs no egress — closed-contour installations work with
tokens alone. For team sign-in, point `[auth.oidc]` at any OIDC issuer
with standard discovery (Keycloak, Authentik, Dex, GitLab, Forgejo…);
GitHub is the one built-in special case (it speaks OAuth2, not OIDC):

```toml
[auth.oidc]
provider = "github"        # or any name for a generic issuer ("keycloak", …)
# issuer = "https://sso.example.com/realms/main"   # generic providers only
client_id = "…"
client_secret = "…"
public_url = "https://converge.example.com"  # register {public_url}/auth/callback
allowed = ["singulared"]   # optional handle allowlist; absent = the IdP decides
```

The login screen then offers "Sign in with …" beside the token paste.
Every authenticated user currently sees everything — multi-user today
means a shared-trust team; per-group authorization is a later milestone.

## Development

Rust 1.88+. Docker is needed for the dev database and integration tests,
never for building (queries compile against the committed `.sqlx/` cache).

```sh
cargo check                 # no database needed — queries check against .sqlx/
cargo test                  # integration tests boot throwaway Postgres containers

cargo xtask dev             # the full stack: Postgres + server, dev token printed
cargo xtask dev --web       # …plus the web app, served same-origin
cargo xtask db              # just the dev Postgres: boots, migrates, prints DATABASE_URL
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
