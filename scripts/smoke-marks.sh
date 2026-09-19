#!/bin/sh
# Smoke of decision marks against a running server: a fresh session's
# start block marks every decision it has never been shown, the receipt
# it leaves behind clears the marks for the next session, and a decision
# recorded after that is the only thing marked new.
#
#   cargo xtask dev
#   scripts/smoke-marks.sh http://127.0.0.1:8080 <token>
set -eu
BASE=${1:?server url}; TOKEN=${2:?bearer token}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cargo build -q -p converge-cli --manifest-path "$ROOT/Cargo.toml"
CONVERGE="$ROOT/target/debug/converge"
S=$(mktemp -d); trap 'rm -rf "$S"' EXIT

id() { python3 -c "import json,sys; print(json.load(sys.stdin)['id'])"; }
api() { m=$1; p=$2; b=${3:-}; if [ -n "$b" ]; then curl -s -X "$m" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d "$b" "$BASE/api/v1$p"; else curl -s -X "$m" -H "Authorization: Bearer $TOKEN" "$BASE/api/v1$p"; fi; }
GID=$(api POST /groups '{"name":"marks","kind":"shared"}' | id)
PID=$(api POST /projects "{\"group_id\":\"$GID\",\"name\":\"marks-p\"}" | id)
decision() { api POST /decisions "{\"project_id\":\"$PID\",\"status\":\"accepted\",\"title\":\"$1\",\"summary\":\"s\",\"context\":null,\"consequences\":null}" > /dev/null; }
decision "first decision"; decision "second decision"

mkdir -p "$S/home/.config/converge" "$S/state" "$S/repo"
printf 'server = "%s"\ntoken = "%s"\n' "$BASE" "$TOKEN" > "$S/home/.config/converge/cli.toml"
printf 'project_id = "%s"\n' "$PID" > "$S/repo/.converge"
export HOME="$S/home" XDG_CONFIG_HOME="$S/home/.config" XDG_STATE_HOME="$S/state"

start() { printf '{"cwd":"%s","session_id":"%s","hook_event_name":"SessionStart"}' "$S/repo" "$1" | "$CONVERGE" hook inject --harness claude; }
show() { python3 -c "
import json,sys
d=json.load(sys.stdin); ctx=d['hookSpecificOutput']['additionalContext']
marked=[l for l in ctx.splitlines() if l.startswith('- ') and 'NEW' in l]
print('  line:', d['systemMessage'].split(' · ')[0])
print('  marked new:', [m.split(' [')[0][2:] for m in marked])"; }

echo "== 1. a first session start: both decisions are new to this reader"
start s-1 | show
echo "== 2. a second session, same person: the receipts cleared the marks"
start s-2 | show
echo "== 3. one more decision, then a third session: only that one is new"
decision "third decision"
start s-3 | show
echo "== 4. the drain empties a backlog in order, off the prompt"
# A transcript the server has never seen: the poll spawns a drain, which
# sends it oldest first in batches and stops when there is nothing left.
T="$S/repo/transcript.jsonl"
i=1; while [ "$i" -le 120 ]; do
  printf '{"type":"user","sessionId":"drain-1","cwd":"/repo","timestamp":"2026-09-19T10:00:00Z","message":{"content":"turn %s"}}\n' "$i" >> "$T"
  i=$((i + 1))
done
poll() { printf '{"cwd":"%s","session_id":"drain-1","hook_event_name":"UserPromptSubmit","transcript_path":"%s"}' "$S/repo" "$T" | "$CONVERGE" hook poll --harness claude >/dev/null; }
recorded() { api GET "/sessions?project=$PID" | python3 -c "
import json,sys
d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
s=[x for x in items if x['external']=='drain-1']
print(len(s) and s[0]['id'] or '')"; }
lift() { python3 -c "
import json; p='$XDG_STATE_HOME/converge/poll.json'; d=json.load(open(p)); d['drain-1']['at']=0; json.dump(d,open(p,'w'))" 2>/dev/null || true; }
for pass in 1 2 3; do
  poll; sleep 3; lift
  sid=$(recorded)
  if [ -n "$sid" ]; then
    n=$(api GET "/sessions/$sid/messages?limit=500" | python3 -c "
import json,sys; d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
print(len(items), '|', (items[0]['body'][:8] if items else ''), '→', (items[-1]['body'][:8] if items else ''))")
    echo "  after pass $pass: $n"
  else
    echo "  after pass $pass: session not opened yet"
  fi
done

echo "== 5. the unseen filter agrees with the block (after three reads, nothing)"
api GET "/decisions?unseen=true" | python3 -c "
import json,sys; d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
print('  unseen over REST:', [i['title'] for i in items])"
