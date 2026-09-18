# Task: complete a cited `path:lines` into a code anchor in `hook ctx`

Status: open. Everything around it is built and tested on
`feat/evidence-anchors`; this is the one missing piece of the evidence slice.

## What exists

- `converge_storage::CodeAnchor { commit, path, lines, excerpt, digest }` with
  `CodeAnchor::validate()` (full 40-hex sha, repository-relative path, 1-based
  inclusive range of at most `EXCERPT_LINES` = 120 lines, non-empty excerpt
  whose sha256 equals `digest`) and `CodeAnchor::digest_of(excerpt)`.
- The server stores anchors on `decision_add` (REST and MCP) and refuses an
  invalid one. The MCP door requires at least one anchor of either kind.
- `crates/converge-cli/src/hook.rs`, `ctx()`: before a `decision_add` the hook
  already records the transcript and cites the new turns. For code it calls
  `drop_incomplete_code(&mut merged)`, which removes every `code_evidence`
  item lacking any of `commit`, `path`, `lines`, `excerpt`, `digest` and
  reports the count on the visible line ("N code citation(s) dropped — cite
  committed anchors in full"). That call is the seam to replace.

## What to build

A resolver in the CLI, suggested as `crates/converge-cli/src/evidence.rs`:

    pub struct Cited { pub path: String, pub lines: (u32, u32) }
    pub enum Resolved { Anchor(CodeAnchor), Refused(String) }
    pub fn resolve(cwd: &Path, cited: &Cited) -> Resolved

Given the working directory the hook runs in and a citation as the model
wrote it, either produce the full anchor or refuse it with a reason a person
can act on. The rules, decided with the user (decisions
`01M2TJMCQ8KJAPVWJ2JJESTW7T` and `01M2TMXDNTTF68HGNG8XKTGM0K`):

1. **Repository and commit.** Find the repository root containing `cwd` and
   its HEAD commit. Not inside a repository → refuse.
2. **Path normalization.** The model cites a path as it sees it: relative to
   `cwd` or to the repository root. Resolve to a repository-relative path with
   forward slashes; that is what the anchor carries.
3. **Tracked only.** An untracked file → refuse ("not tracked — commit first").
4. **Excerpt from the commit, not the working tree.** Read the cited range
   from the file *as it is at HEAD*. A range past the end of the file →
   refuse.
5. **Reproducibility.** Compare the same range in the working tree with the
   HEAD content. Any difference → refuse ("uncommitted changes in
   `path:a-b` — commit first"). Only the cited range counts; unrelated edits
   elsewhere in the file do not block.
6. **Limits.** `lines` 1-based, inclusive, start ≤ end, at most 120 lines;
   otherwise refuse ("split it").
7. **Fill.** `commit` = HEAD's full sha, `excerpt` = the cited lines joined
   with `\n` plus a trailing newline, `digest` = `CodeAnchor::digest_of`.
   The result must pass `CodeAnchor::validate()`.
8. **Submodules.** A path inside a submodule belongs to the submodule's
   repository: its root, its HEAD. (Nearest-repository rule; no special
   casing beyond running the lookups from the file's own directory.)

Refusal messages are shown to the person on the hook's visible line, so they
say what to do, not what went wrong internally.

## Wiring

In `ctx()`, for each `code_evidence` item that has `path` and `lines` but no
`commit`: call `resolve(&payload.cwd, ..)`; replace the item with the anchor
on `Resolved::Anchor`, remove it and add the reason to `notes` on
`Resolved::Refused`. Items that already carry every field pass through
untouched (an importer or the model may send full anchors). Keep
`drop_incomplete_code` for whatever is left malformed, or fold it in.

The whole enrichment runs inside `CTX_BUDGET` (4 s); a few git invocations
are far below that.

## Tests

Unit tests in the resolver's module, each against a temporary repository
created in the test (initialize, add a file, commit):

- a clean cited range → an anchor with HEAD's sha, the exact lines, a digest
  that validates;
- the same range after an uncommitted edit inside it → refused;
- an uncommitted edit *outside* the range → still anchored;
- an untracked file → refused;
- a range past the end, a reversed range, a 121-line range → refused with
  the matching reason;
- a path cited relative to a subdirectory `cwd` → normalized to the
  repository-relative path.

Plus one `hook.rs` test for the wiring: a call with one bare citation and one
full anchor produces an input with two anchors and no note.

## Done when

`converge hook ctx --harness claude` with a `decision_add` input citing
`{"path": "...", "lines": [a, b]}` for committed lines yields
`updatedInput.code_evidence[0]` that the server accepts, and step 3 of
`scripts/smoke-ctx.sh` (run against `cargo xtask dev`) reports one code anchor
kept and no note. Note that the script's citation names `src/lib.rs` in a
repository it creates without that file; make the bound repository a real
one with a committed file for the positive case.
