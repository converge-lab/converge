#!/bin/sh
# Smoke of `converge hook ctx` before a `decision_add`, against a running
# server: the recent turns ride on the call itself, each at its position
# in the conversation, so the decision is anchored in one round trip and
# the drain that comes later records those turns once. A bare
# `path:lines` citation comes back a full anchor — commit, excerpt,
# digest — filled in from the bound repository's HEAD
# (docs/tasks/hook-code-anchor-resolver.md).
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
RUN="$(date +%s)-$$"   # sessions are keyed by external id, globally

id() { python3 -c "import json,sys; print(json.load(sys.stdin)['id'])"; }
api() { m=$1; p=$2; b=${3:-}; if [ -n "$b" ]; then curl -s -X "$m" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d "$b" "$BASE/api/v1$p"; else curl -s -X "$m" -H "Authorization: Bearer $TOKEN" "$BASE/api/v1$p"; fi; }
GID=$(api POST /groups '{"name":"ctx-smoke","kind":"shared"}' | id)
PID=$(api POST /projects "{\"group_id\":\"$GID\",\"name\":\"ctx-p\"}" | id)

mkdir -p "$S/home/.config/converge" "$S/state" "$S/repo/src"
printf 'server = "%s"\ntoken = "%s"\n' "$BASE" "$TOKEN" > "$S/home/.config/converge/cli.toml"
printf 'project_id = "%s"\n' "$PID" > "$S/repo/.converge"
# A real repository with a committed file: the resolver completes a bare
# citation from HEAD, so a citation needs a HEAD to come from.
printf 'pub fn sessions() -> usize {\n    1\n}\n' > "$S/repo/src/lib.rs"
git -C "$S/repo" init -q
git -C "$S/repo" add src/lib.rs
git -C "$S/repo" -c user.email=smoke@converge -c user.name=Smoke -c commit.gpgsign=false commit -qm 'ctx smoke fixture'
T="$S/repo/transcript.jsonl"
SID="ctx-$RUN"
cat > "$T" <<EOF
{"type":"user","sessionId":"$SID","cwd":"/repo","timestamp":"2026-09-18T10:00:00Z","message":{"content":"should sessions live in redb?"}}
{"type":"assistant","sessionId":"$SID","timestamp":"2026-09-18T10:00:05Z","message":{"content":[{"type":"text","text":"Postgres, one backend for everything."}]}}
{"type":"user","sessionId":"$SID","timestamp":"2026-09-18T10:00:09Z","message":{"content":"agreed, record it"}}
EOF
export HOME="$S/home" XDG_CONFIG_HOME="$S/home/.config" XDG_STATE_HOME="$S/state"

ctx() { printf '{"cwd":"%s","session_id":"%s","hook_event_name":"PreToolUse","tool_name":"mcp__converge__decision_add","transcript_path":"%s","tool_input":%s}' "$S/repo" "$SID" "$T" "$1" | "$CONVERGE" hook ctx --harness claude; }
# What the hook put on the call, and what the model had already written.
show() { python3 -c "
import json,sys
d=json.load(sys.stdin); u=d['hookSpecificOutput']['updatedInput']
turns=u.get('evidence_turns',[])
print('  conversation:', (u.get('conversation') or {}).get('external'))
print('  turns cited:', len(turns), '| ordinals:', [t['ordinal'] for t in turns])
print('  code anchors:', len(u.get('code_evidence',[])), '| system:', d.get('systemMessage'))
json.dump(u, open('$S/call.json','w'))"; }
# The call as the hook left it, sent the way the agent would send it.
record() { python3 -c "
import json; u=json.load(open('$S/call.json'))
print(json.dumps({'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':'decision_add','arguments':u}}))" \
  | curl -s -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
         -H "Accept: application/json, text/event-stream" -d @- "$BASE/mcp" \
  | python3 -c "
import json,sys
d=json.load(sys.stdin)
if 'error' in d: print('  refused:', d['error']['message']); raise SystemExit
r=json.loads(d['result']['content'][0]['text']); print('  decision:', r['decision_id'])
open('$S/decision','w').write(r['decision_id'])"; }

echo "== 1. a decision_add: the recent turns ride on the call, no network"
ctx "{\"project_id\":\"$PID\",\"title\":\"Store sessions in Postgres\",\"summary\":\"one backend\"}" | show
record
echo "== 2. one more turn, one bare code citation — completed from HEAD, one anchor kept, no note"
cat >> "$T" <<EOF
{"type":"assistant","sessionId":"$SID","timestamp":"2026-09-18T10:01:00Z","message":{"content":[{"type":"text","text":"Recording it now."}]}}
EOF
ctx "{\"project_id\":\"$PID\",\"title\":\"t\",\"summary\":\"s\",\"code_evidence\":[{\"path\":\"src/lib.rs\",\"lines\":[1,2]}]}" | show
echo "== 3. the decision cites the exchange the model was having"
DID=$(cat "$S/decision")
api GET "/decisions/$DID/sources" | python3 -c "
import json,sys
d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
print('  anchored turns:', len(items[0]['anchors']))
print('  on record:', [m['body'][:24] for m in items[0]['messages']])"
echo "== 4. the sync sends the whole transcript: no second copy of those turns"
printf '{"cwd":"%s","session_id":"%s","hook_event_name":"SessionEnd","transcript_path":"%s"}' "$S/repo" "$SID" "$T" | "$CONVERGE" hook sync --harness claude > /dev/null
api GET "/sessions?project=$PID" | python3 -c "
import json,sys
d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
print('  sessions:', len(items), '| title:', items[0]['title'] if items else None)
open('$S/sid','w').write(items[0]['id'] if items else '')"
api GET "/sessions/$(cat "$S/sid")/messages?limit=100" | python3 -c "
import json,sys
d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
print('  turns on record:', len(items), '|', [m['body'][:24] for m in items])"
