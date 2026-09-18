#!/bin/sh
# Smoke of `converge hook ctx` before a `decision_add`, against a running
# server: the transcript goes up and the new turns are cited; a repeat
# cites nothing new; a bare code citation is dropped and said until the
# resolver lands (docs/tasks/hook-code-anchor-resolver.md), after which
# step 3 should report it kept.
#
#   cargo xtask dev            # prints the server URL and a token
#   scripts/smoke-ctx.sh http://127.0.0.1:8080 <token>
#
# Everything lives under a temporary directory: HOME, the state dir, the
# bound repository and the transcript. Nothing on the machine is touched.
set -eu
BASE=${1:?server url}; TOKEN=${2:?bearer token}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cargo build -q -p converge-cli --manifest-path "$ROOT/Cargo.toml"
CONVERGE="$ROOT/target/debug/converge"
S=$(mktemp -d); trap 'rm -rf "$S"' EXIT

id() { python3 -c "import json,sys; print(json.load(sys.stdin)['id'])"; }
api() { m=$1; p=$2; b=${3:-}; if [ -n "$b" ]; then curl -s -X "$m" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d "$b" "$BASE/api/v1$p"; else curl -s -X "$m" -H "Authorization: Bearer $TOKEN" "$BASE/api/v1$p"; fi; }
GID=$(api POST /groups '{"name":"ctx-smoke","kind":"shared"}' | id)
PID=$(api POST /projects "{\"group_id\":\"$GID\",\"name\":\"ctx-p\"}" | id)

mkdir -p "$S/home/.config/converge" "$S/state" "$S/repo"
printf 'server = "%s"\ntoken = "%s"\n' "$BASE" "$TOKEN" > "$S/home/.config/converge/cli.toml"
printf 'project_id = "%s"\n' "$PID" > "$S/repo/.converge"
T="$S/repo/transcript.jsonl"
printf '%s\n' \
 '{"type":"user","sessionId":"ctx-1","cwd":"/repo","timestamp":"2026-09-18T10:00:00Z","message":{"content":"should sessions live in redb?"}}' \
 '{"type":"assistant","sessionId":"ctx-1","timestamp":"2026-09-18T10:00:05Z","message":{"content":[{"type":"text","text":"Postgres — one backend for everything."}]}}' \
 '{"type":"user","sessionId":"ctx-1","timestamp":"2026-09-18T10:00:09Z","message":{"content":"agreed, record it"}}' > "$T"
export HOME="$S/home" XDG_CONFIG_HOME="$S/home/.config" XDG_STATE_HOME="$S/state"

ctx() { printf '{"cwd":"%s","session_id":"ctx-1","hook_event_name":"PreToolUse","tool_name":"mcp__converge__decision_add","transcript_path":"%s","tool_input":%s}' "$S/repo" "$T" "$1" | "$CONVERGE" hook ctx --harness claude; }
show() { python3 -c "import json,sys; d=json.load(sys.stdin); u=d['hookSpecificOutput']['updatedInput']; print('evidence ids:', len(u.get('evidence',[])), '| code anchors:', len(u.get('code_evidence',[])), '| system:', d.get('systemMessage'))"; }

echo "== 1. first decision_add: the three turns go up and are cited"
ctx "{\"project_id\":\"$PID\",\"title\":\"Store sessions in Postgres\",\"summary\":\"one backend\"}" | show
echo "== 2. same transcript again: nothing new to cite"
ctx "{\"project_id\":\"$PID\",\"title\":\"again\",\"summary\":\"s\"}" | show
echo "== 3. one more turn, one bare code citation (kept once the resolver lands)"
printf '%s\n' '{"type":"assistant","sessionId":"ctx-1","timestamp":"2026-09-18T10:01:00Z","message":{"content":[{"type":"text","text":"Recording it now."}]}}' >> "$T"
ctx "{\"project_id\":\"$PID\",\"title\":\"t\",\"summary\":\"s\",\"code_evidence\":[{\"path\":\"src/lib.rs\",\"lines\":[1,2]}]}" | show
echo "== 4. the server holds the session"
api GET "/sessions?project=$PID" | python3 -c "import json,sys; d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d; print('sessions:', len(items), '| title:', items[0]['title'] if items else None)"
