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

want() { if [ "$2" = "$3" ]; then echo "  ok: $1 = $3"; else echo "  FAIL: $1 — wanted [$2], got [$3]"; exit 1; fi; }
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
# What the hook put on the call, as one comparable line, and the call
# itself saved for the step that sends it.
show() { python3 -c "
import json,sys
d=json.load(sys.stdin); u=d['hookSpecificOutput']['updatedInput']
turns=u.get('evidence_turns',[])
json.dump(u, open('$S/call.json','w'))
print('%s, ordinals %s, %d anchor(s), said %s' % (
    (u.get('conversation') or {}).get('external'),
    ','.join(str(t['ordinal']) for t in turns) or 'none',
    len(u.get('code_evidence',[])),
    d.get('systemMessage') or 'nothing'))"; }
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
want "the call" "$SID, ordinals 0,1,2, 0 anchor(s), said nothing" \
     "$(ctx "{\"project_id\":\"$PID\",\"title\":\"Store sessions in Postgres\",\"summary\":\"one backend\"}" | show)"
record
echo "== 2. one more turn, one bare code citation — completed from HEAD, one anchor kept, no note"
cat >> "$T" <<EOF
{"type":"assistant","sessionId":"$SID","timestamp":"2026-09-18T10:01:00Z","message":{"content":[{"type":"text","text":"Recording it now."}]}}
EOF
want "the call" "$SID, ordinals 0,1,2,3, 1 anchor(s), said nothing" \
     "$(ctx "{\"project_id\":\"$PID\",\"title\":\"t\",\"summary\":\"s\",\"code_evidence\":[{\"path\":\"src/lib.rs\",\"lines\":[1,2]}]}" | show)"
echo "== 3. the decision cites the exchange the model was having"
DID=$(cat "$S/decision")
anchored=$(api GET "/decisions/$DID/sources" | python3 -c "
import json,sys
d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
print('%d anchored, %d on record' % (len(items[0]['anchors']), len(items[0]['messages'])))")
want "the decision's sources" "3 anchored, 3 on record" "$anchored"
echo "== 4. the sync sends the whole transcript: no second copy of those turns"
printf '{"cwd":"%s","session_id":"%s","hook_event_name":"SessionEnd","transcript_path":"%s"}' "$S/repo" "$SID" "$T" | "$CONVERGE" hook sync --harness claude > /dev/null
sessions=$(api GET "/sessions?project=$PID" | python3 -c "
import json,sys
d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
open('$S/sid','w').write(items[0]['id'] if items else '')
print(len(items), 'session(s),', items[0]['title'] if items else 'none')")
want "sessions" "1 session(s), should sessions live in redb?" "$sessions"
recorded=$(api GET "/sessions/$(cat "$S/sid")/messages?limit=100" | python3 -c "
import json,sys
d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
print('%d turns, %s → %s' % (len(items), items[0]['body'][:15].strip(), items[-1]['body'][:16].strip()))")
want "turns on record" "4 turns, should sessions → Recording it now" "$recorded"

echo "== 5. a decision early in a long session: the sync still records what came before it"
# The cited turns land at their own positions with nothing below them.
# A resume point of "highest position plus one" would skip the start of
# the conversation for good; the first hole is what it must answer.
T2="$S/repo/long.jsonl"
SID2="ctx-long-$RUN"
i=0; while [ "$i" -lt 30 ]; do
  cat >> "$T2" <<EOF
{"type":"user","sessionId":"$SID2","cwd":"/repo","timestamp":"2026-09-20T11:00:00Z","message":{"content":"long turn $i"}}
EOF
  i=$((i + 1))
done
printf '{"cwd":"%s","session_id":"%s","hook_event_name":"PreToolUse","tool_name":"mcp__converge__decision_add","transcript_path":"%s","tool_input":%s}' \
  "$S/repo" "$SID2" "$T2" "{\"project_id\":\"$PID\",\"title\":\"decided early\",\"summary\":\"s\"}" \
  | "$CONVERGE" hook ctx --harness claude | show > /dev/null
record
printf '{"cwd":"%s","session_id":"%s","hook_event_name":"SessionEnd","transcript_path":"%s"}' "$S/repo" "$SID2" "$T2" | "$CONVERGE" hook sync --harness claude > /dev/null
long=$(api GET "/sessions?project=$PID" | python3 -c "
import json,sys
d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
s=[x for x in items if x['external']=='$SID2']
print(s[0]['id'] if s else '')")
recorded=$(api GET "/sessions/$long/messages?limit=100" | python3 -c "
import json,sys
d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
print('%d turns, %s → %s' % (len(items), items[0]['body'], items[-1]['body']))")
want "the whole conversation" "30 turns, long turn 0 → long turn 29" "$recorded"
