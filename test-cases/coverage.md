# Converge Test Cases

## Already covered

### Groups & Projects
- [x] Create a group → read it back (`group_round_trip`, storage-postgres/groups_projects.rs)
- [x] List groups is newest-first (`group_round_trip`, storage-postgres/groups_projects.rs)
- [x] Edit name / clear description (`group_round_trip`, storage-postgres/groups_projects.rs)
- [x] Unknown group id → `None` on get, `NotFound` on edit (`group_round_trip`, storage-postgres/groups_projects.rs)
- [x] Create a project → read it back (`project_round_trip`, storage-postgres/groups_projects.rs)
- [x] Filter projects by group, newest-first (`project_round_trip`, storage-postgres/groups_projects.rs)
- [x] Project list `limit` and cursor paging (`project_round_trip`, storage-postgres/groups_projects.rs)
- [x] Edit batch on a project (`project_round_trip`, storage-postgres/groups_projects.rs)
- [x] Unknown `group_id` on project add → `Invalid` (FK) (`project_round_trip`, storage-postgres/groups_projects.rs)
- [x] Unknown project id on edit → `NotFound` (`project_round_trip`, storage-postgres/groups_projects.rs)
- [x] Group CRUD over REST, incl. RFC3339 timestamps and unknown-id 404 shape (`group_crud`, server/api.rs)
- [x] Project CRUD over REST, `?group=` filter, `?limit=` + `next_cursor` (`project_crud`, server/api.rs)
- [x] Creating a project with unknown `group_id` over REST → 400 `invalid` (`project_crud`, server/api.rs)
- [x] Cursor pagination is lossless and non-overlapping across pages (`pagination`, server/api.rs)

### Users, Agents, Authorship
- [x] `user_login` upserts by `(provider, subject)`; handle/name refresh on re-login (`ensure_by_natural_key`, storage-postgres/authors.rs)
- [x] Different subject or provider → different user id (`ensure_by_natural_key`, storage-postgres/authors.rs)
- [x] `agent_ensure` upserts by `(kind, name)`; different kind → different agent (`ensure_by_natural_key`, storage-postgres/authors.rs)
- [x] Unknown user/agent id → `None` on get (`ensure_by_natural_key`, storage-postgres/authors.rs)
- [x] `user_list` / `agent_list` respect `Pagination{limit}` (`ensure_by_natural_key`, storage-postgres/authors.rs)
- [x] All three `Author` variants (`User`, `Agent`, `UserViaAgent`) round-trip on a decision (`authorship_round_trip`, storage-postgres/authors.rs)
- [x] Duplicate authors on a decision collapse to one (`authorship_round_trip`, storage-postgres/authors.rs)
- [x] Unknown author on decision add → `Invalid`, no partial row left (`unknown_author_rejected`, storage-postgres/authors.rs)
- [x] `GET /users/me` auto-provisions the configured admin identity, deterministic id across calls (`users_me`, server/api.rs)
- [x] `GET /agents` starts empty, reflects an agent created via `agent_ensure` (`users_me`, server/api.rs)

### Decisions — fields, CRUD, atomicity
- [x] Create a decision → read all core fields back (`round_trip`, storage-postgres/decisions.rs)
- [x] Unknown decision id → `None` (`round_trip`, storage-postgres/decisions.rs)
- [x] List: default newest-first, `project` filter, `group` filter, `status` filter, `limit`, cursor paging (`list_filters`, storage-postgres/decisions.rs)
- [x] Edit batch applies atomically; untouched fields survive; unknown id → `NotFound` (`edit_batch`, storage-postgres/decisions.rs)
- [x] A batch where one op fails (NUL byte) rolls back the whole batch (`edit_batch_is_atomic`, storage-postgres/decisions.rs)
- [x] Unknown author or unknown `project_id` on add → `Invalid` (`add_guards`, storage-postgres/decisions.rs)
- [x] Create a decision with status `Proposed` → read → verify status (`add_with_proposed_status`, storage-postgres/decisions.rs)
- [x] Create a decision with status `Rejected` → read → verify status (`add_with_rejected_status`, storage-postgres/decisions.rs)
- [x] Create a decision → read all fields back over REST, incl. full `authors`/`alternatives` (`decision_crud`, server/decision.rs)
- [x] Minimal create defaults collections to `[]` on the wire (`decision_crud`, server/decision.rs)
- [x] PATCH batch (`set_status`, `set_context`) reflected on re-GET (`decision_crud`, server/decision.rs)
- [x] `?project=`, `?group=&status=&limit=` filters compose over REST (`decision_crud`, server/decision.rs)
- [x] Creating with `status: "superseded"` → 400 "derived" (`decision_crud`, server/decision.rs)
- [x] Unknown id → 404 on GET and PATCH (`decision_crud`, server/decision.rs)
- [x] `/groups/{id}/projects` and `/projects/{id}/decisions?status=` nested projections mirror flat filters (`relation_projections`, server/decision.rs)
- [x] Missing parent (group/project) on a nested feed → 404, not empty list (`relation_projections`, server/decision.rs)
- [x] Re-specifying the path-bound parent via query string → 400 "bound by the path" (`relation_projections`, server/decision.rs)

### Decision graph (supersession, related)
- [x] Supersession derives `Superseded` on the old decision without storing it; edges both directions (`supersession_derives_status`, storage-postgres/decisions.rs)
- [x] Status filter matches the derived status (`supersession_derives_status`, storage-postgres/decisions.rs)
- [x] Removing the last inbound supersede edge restores the stored status (`supersession_derives_status`, storage-postgres/decisions.rs)
- [x] `decision_edges` on unknown id → `None` (`supersession_derives_status`, storage-postgres/decisions.rs)
- [x] `AddRelated` is an upsert (re-add updates `why`, no duplicate), reflected on both sides (`related_upsert`, storage-postgres/decisions.rs)
- [x] `RemoveRelated` is idempotent (`related_upsert`, storage-postgres/decisions.rs)
- [x] Self-loop `AddSupersedes`/`AddRelated` → `Invalid` (`graph_guards`, storage-postgres/decisions.rs)
- [x] `SetStatus(Superseded)` and creating already-`Superseded` → `Invalid` (`graph_guards`, storage-postgres/decisions.rs)
- [x] A creation-time `supersedes` edge to a nonexistent decision → `Invalid`, whole decision not created (`graph_guards`, storage-postgres/decisions.rs)
- [x] Supersession over REST derives status on both GET and filtered list (`decision_graph`, server/decision.rs)
- [x] `/decisions/{id}/edges` carries both directions over REST (`decision_graph`, server/decision.rs)
- [x] `add_related`/`why` round-trips from both `related_to` and `related_by` over REST (`decision_graph`, server/decision.rs)
- [x] Self-loop `add_supersedes` over REST → 400 (`decision_graph`, server/decision.rs)
- [x] Unknown decision on `/edges` → 404 (`decision_graph`, server/decision.rs)

### Evidence, Sessions, Messages
- [x] `session_ensure` upserts by `(kind, external)`; title refreshes, project binding stays as first-created (`session_ensure_by_natural_key`, storage-postgres/evidence.rs)
- [x] Same external id under a different kind → different session (`session_ensure_by_natural_key`, storage-postgres/evidence.rs)
- [x] `SessionFilter{project, kind}` narrows; unknown session id → `None` (`session_ensure_by_natural_key`, storage-postgres/evidence.rs)
- [x] Message batches append with contiguous increasing `seq`, order preserved (`streams_append_in_order`, storage-postgres/evidence.rs)
- [x] Explicit `sent_at` preserved distinct from server `captured_at` (`streams_append_in_order`, storage-postgres/evidence.rs)
- [x] Forward cursor pagination on messages; add to unknown session → `NotFound`; list on unknown session → empty (`streams_append_in_order`, storage-postgres/evidence.rs)
- [x] Evidence anchors round-trip via get/list; duplicate anchors collapse (`evidence_anchors_decisions_to_messages`, storage-postgres/evidence.rs)
- [x] `AddEvidence`/`RemoveEvidence` grow/shrink the set; unknown message → `Invalid` (`evidence_anchors_decisions_to_messages`, storage-postgres/evidence.rs)
- [x] `decision_sources` groups by session (newest first) and computes anchor windows correctly, incl. union of disjoint windows (`sources_derive_windows_around_anchors`, storage-postgres/evidence.rs)
- [x] No-evidence decision → `Some(empty)`; unknown decision → `None` (`sources_derive_windows_around_anchors`, storage-postgres/evidence.rs)
- [x] Session ensure/get/filter, message append + pagination, anchoring, and `/decisions/{id}/sources` all over REST (`evidence_over_rest`, server/evidence.rs)
- [x] Unknown session/decision ids on the REST session/message/sources routes → 404 (`evidence_over_rest`, server/evidence.rs)

### Search
- [x] Stemming matches base forms (`ranked_stemmed_and_filtered`, storage-postgres/search.rs)
- [x] Ranking weight order: title > summary > context (`ranked_stemmed_and_filtered`, storage-postgres/search.rs)
- [x] `project` filter composes with search (`ranked_stemmed_and_filtered`, storage-postgres/search.rs)
- [x] Websearch syntax: `-exclusion`, `"quoted phrase"` (`ranked_stemmed_and_filtered`, storage-postgres/search.rs)
- [x] `limit` caps ranked results from the top (`ranked_stemmed_and_filtered`, storage-postgres/search.rs)
- [x] Whitespace-only or all-operator query → `Invalid` (`ranked_stemmed_and_filtered`, storage-postgres/search.rs)
- [x] Query with no matches → empty (not an error) (`ranked_stemmed_and_filtered`, storage-postgres/search.rs)
- [x] `?q=` over REST, stemmed, unpaged; `?q=` + `?cursor=` together → 400; term-free query → 400 (`search_rides_the_list`, server/decision.rs)

### Signals
- [x] Duplicate targets collapse to a set; all fields round-trip, born `Proposed` (`round_trip_and_invariants`, storage-postgres/signals.rs)
- [x] Empty targets, target == source, whitespace-only kind → `Invalid` (`round_trip_and_invariants`, storage-postgres/signals.rs)
- [x] Unknown source decision on add → `Invalid`; unknown id on get → `None` (`round_trip_and_invariants`, storage-postgres/signals.rs)
- [x] Overlapping targets on the same `(source, *, kind)` → `Conflict`, even after dismissal (`pairs_are_never_re_raised`, storage-postgres/signals.rs)
- [x] A different `kind` for the same source/target is allowed (`pairs_are_never_re_raised`, storage-postgres/signals.rs)
- [x] Resolving to `Proposed` → `Invalid`; resolving stamps `resolved_by`; re-resolving flips the verdict; unknown id → `NotFound` (`resolution_stamps_the_judge`, storage-postgres/signals.rs)
- [x] `SignalFilter{project}`/`{decision}` match either the source or a target; `tier` and `status` filters narrow (`list_filters_match_either_end`, storage-postgres/signals.rs)
- [x] Record via REST → 201 `proposed`; GET reflects fields (`signal_round_trip`, server/signal.rs)
- [x] Duplicate pair over REST → 409 (`signal_round_trip`, server/signal.rs)
- [x] List narrows by `?decision=&tier=`; `/decisions/{id}/signals` projection; unknown decision → 404 (`signal_round_trip`, server/signal.rs)
- [x] PATCH resolve to `confirmed` → 204, `resolved_by` stamped; resolve to `proposed` → 400 (`signal_round_trip`, server/signal.rs)

### Tokens
- [x] Mint → resolve by hash; unknown hash → `None` (`lifecycle_and_owner_scoping`, storage-postgres/tokens.rs)
- [x] `token_list` scoped per-owner (`lifecycle_and_owner_scoping`, storage-postgres/tokens.rs)
- [x] Non-owner revoke → `NotFound`, credential still works (`lifecycle_and_owner_scoping`, storage-postgres/tokens.rs)
- [x] Owner revoke kills the credential; revoking twice → `NotFound` (`lifecycle_and_owner_scoping`, storage-postgres/tokens.rs)
- [x] Mint over REST → 201, secret shown once, authenticates immediately (`token_lifecycle_over_rest`, server/token.rs)
- [x] List never leaks secrets, only labels (`token_lifecycle_over_rest`, server/token.rs)
- [x] DELETE (revoke) → 204, secret stops working, double-revoke → 404 (`token_lifecycle_over_rest`, server/token.rs)

### Auth — session cookies, OIDC sign-in, MCP OAuth connector
- [x] Bearer → session cookie exchange (`HttpOnly`, `SameSite=Strict`); cookie-only auth resolves the owner; logout expires the cookie (`session_round_trip`, server/session.rs)
- [x] Unknown token → 401; forged cookie → 401, not a crash (`bad_credentials_stay_out`, server/session.rs)
- [x] OIDC login redirect carries `state` + flow cookie; callback exchanges code with PKCE verifier, calls userinfo, issues session cookie (`sign_in_round_trip`, server/signin.rs)
- [x] Tampered `state` on callback → 400 before token exchange (`sign_in_round_trip`, server/signin.rs)
- [x] Allowlist excluding the identity → 403, no session cookie set (`allowlist_turns_the_identity_away`, server/signin.rs)
- [x] Discovery documents reflect request `Host`; dynamic client registration; PKCE authorize/token/refresh/revoke round trip (`connector_round_trip`, server/connector.rs)
- [x] Access token authenticates REST and MCP; refresh token shows up in `token_list`; revoking it kills future refreshes (`connector_round_trip`, server/connector.rs)
- [x] Unregistered `redirect_uri` and missing PKCE challenge → 400 (`authorize_rejects_what_the_code_would_leak`, server/connector.rs)

### MCP tool surface
- [x] `tools/list` returns exactly the 10 blessed tools (`tool_round_trip`, server/mcp.rs)
- [x] `decision_add`/`get`/`list`/`search` over MCP, incl. supersede and derived status (`tool_round_trip`, server/mcp.rs)
- [x] MCP-originated decisions are authored `user_via_agent` with an auto-ensured tool agent (`tool_round_trip`, server/mcp.rs)
- [x] Malformed argument surfaces as an MCP-level error, not a crash (`tool_round_trip`, server/mcp.rs)
- [x] `session_ensure`/`message_add`/anchored `decision_add` via MCP, reflected in `/decisions/{id}/sources` (`ingest_round_trip`, server/mcp.rs)
- [x] `project_match` ranks by cwd/remote hint; `project_bind` by id or by name (auto-picks sole group); `project_dismiss` scope semantics (`mapping_round_trip`, server/mcp.rs)

### Health & routing
- [x] `/healthz` works without auth; auth gate rejects missing/wrong tokens and unknown paths alike (401 before 404) (`healthz`, server/health.rs)
- [x] SPA fallback serves `index.html` for unmatched paths while API routes still resolve as API (`healthz`, server/health.rs)

### Client SDK (`converge-client`)
- [x] Version check, stable identity, full token mint/auth/list/revoke/re-revoke lifecycle (`round_trip`, client.rs)
- [x] Group/project/decision CRUD, supersede-at-creation, evidence, sources, session/message streaming, cursor walk with no overlap/loss (`round_trip`, client.rs)
- [x] Errors map to domain types: `NotFound`, `Invalid` (with server message), `Unauthorized` on a revoked token, `Unavailable` on dead server (`errors_map_back_to_the_domain`, client.rs)

### Expert — signal discovery job
- [x] Golden fixture deserializes and names the right players (`the_fixture_parses_and_names_the_players`, expert/signals.rs)
- [x] A bogus target id in the model's reply is dropped, real target kept; `kind` normalized to snake_case; request pins `temperature: 0.0` + schema-constrained JSON (`the_job_constrains_the_wire_and_validates_the_reply`, expert/signals.rs)
- [x] An empty `signals: []` reply is a success, not an error (`an_empty_judgment_is_a_success`, expert/signals.rs)
- [x] The model actually flags a known conflict end-to-end (`live_model_finds_the_sse_conflict`, expert/signals.rs) — `#[ignore]`d, needs a live model endpoint (`CONVERGE_EXPERT_URL`/`CONVERGE_EXPERT_MODEL`), not part of normal CI

### Expert — model wire client
- [x] OpenAI-compatible reply extraction and trimming (`openai_compat_round_trip`, expert/wire.rs)
- [x] Anthropic-shaped request carries the resolved API key in `x-api-key` (`anthropic_round_trip_carries_the_key`, expert/wire.rs)
- [x] A 500 from the model endpoint is a typed error, not a panic (`http_failure_is_a_typed_error_not_a_panic`, expert/wire.rs)
- [x] The request timeout is a hard bound, not best-effort (`the_budget_is_a_hard_bound`, expert/wire.rs)

---

## Missing coverage

### Decisions
- [ ] Set and read back a non-`None` `consequences` value, both on `decision_add` and via `DecisionEdit::SetConsequences` (the only `DecisionEdit` variant with zero test coverage today)
- [ ] `Related.why: None` (as opposed to `Some(...)`) round-trips
- [ ] `list_views`/list with a totally empty store returns `[]` cleanly (most list tests seed at least one row first)

### Users & Agents
- [ ] Two agents with the same `name` but different `kind` are distinct over the REST surface (only covered at the storage layer)
- [ ] `/api/v1/agents` and `/api/v1/users` pagination (`?limit=`) over REST
- [ ] A second real user (different `provider`/`subject`) interacts with the API — today every server test runs as the single admin identity

### Search
- [ ] `or` operator in websearch syntax (docs mention it; no test exercises it)
- [ ] `?q=` combined with `?status=` over REST (only `project` is combined at the storage layer)

### Signals
- [ ] Signal add with an unknown *target* decision → `Invalid` (only unknown *source* is tested)
- [ ] `SignalFilter` with both `project` and `decision` set together
- [ ] A signal whose source and target live in different projects (cross-project signal) — allowed or rejected is currently untested either way
- [ ] `produced_by`/`resolved_by` using the `Agent` or `UserViaAgent` author variant (only `User` is used throughout)
- [ ] `Tier` ordering (`Watch < Coordinate < Conflict`) is derived but never asserted

### Sessions & pagination edge cases
- [ ] `Session.kind` variants `Pr` and `Incident` (only `Transcript` and `Slack` appear anywhere)
- [ ] `Pagination{limit: Some(0)}` behavior
- [ ] Negative or extremely large `?limit=` over REST
- [ ] A cursor from one resource type used against a different resource's list (garbage/foreign cursor handling)
- [ ] `message_list` cursor and `limit` combined in the same call

### Large / unusual content
- [ ] A decision `summary`/`context` or message `body` at multi-KB/MB size
- [ ] A large batch `message_add` (hundreds of messages in one call)
- [ ] Unicode edge cases (emoji, RTL text) surviving round-trip and full-text search

### Access control / tenant isolation
- [ ] Document (and, if intended, test) that there is currently no per-group visibility boundary — every authenticated caller sees every group's data
- [ ] A second real user editing or reading another user's decision through the HTTP/MCP layer, not just at the storage layer
- [ ] OIDC allowlist acceptance path with a non-default, non-empty allowed list that explicitly includes the caller
- [ ] Whether an MCP connector's minted token is scoped any differently from a regular bearer token (e.g., can it mint/revoke other tokens?)

### Concurrency
- [ ] Concurrent `message_add` batches on the same session don't interleave or collide on `seq` (doc-commented as a guarantee, never tested under real concurrency)
- [ ] Concurrent `session_ensure` / `user_login` / `agent_ensure` on the same natural key resolve to one row, not two
- [ ] Concurrent `signal_add` on the same `(source, target, kind)` pair — exactly one wins, the other sees `Conflict`
- [ ] Concurrent `decision_edit` batches on the same decision

### Wire / serialization
- [ ] Malformed JSON body (truncated, wrong field type, unknown enum tag) on a REST POST/PATCH
- [ ] Full enum coverage over the wire for `DecisionStatus` (`Draft`, `Rejected` — only `proposed`/`accepted`/`superseded` ever cross REST/MCP in any test, incl. the `"draft"`/`"rejected"` string-parse arms in `mcp/mod.rs`), `SessionKind` (`Pr`, `Incident`)
- [ ] `converge-client` error mapping for `Conflict` (e.g., duplicate signal pair) and `Backend` variants — currently the only `Conflict` producer in the whole codebase is the signal duplicate-pair path, and it's never asserted through the typed client, only over raw REST (`signal_round_trip`, server/signal.rs)

### Expert
- [ ] A schema-invalid or non-JSON model reply (only "valid with one bad target" and "empty" are covered)
- [ ] Multiple decisions in one discovery request producing multiple drafts
- [ ] `api_key_cmd` failure (missing command / nonzero exit) surfaces as a typed error
