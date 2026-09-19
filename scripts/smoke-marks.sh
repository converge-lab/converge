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
RUN="$(date +%s)-$$"   # sessions are keyed by external id, globally

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
echo "== 4. session end records the conversation, in order"
# A transcript the server has never seen: the session-end sync sends it
# oldest first and the server keeps one row per position.
T="$S/repo/transcript.jsonl"
SID="archive-$RUN"
i=1; while [ "$i" -le 120 ]; do
  cat >> "$T" <<EOF
{"type":"user","sessionId":"$SID","cwd":"/repo","timestamp":"2026-09-20T10:00:00Z","message":{"content":"turn $i"}}
EOF
  i=$((i + 1))
done
sync_now() { printf '{"cwd":"%s","session_id":"%s","hook_event_name":"SessionEnd","transcript_path":"%s"}' "$S/repo" "$SID" "$T" | "$CONVERGE" hook sync --harness claude; }
recorded() { api GET "/sessions?project=$PID" | python3 -c "
import json,sys
d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
s=[x for x in items if x['external']=='$SID']
print(len(s) and s[0]['id'] or '')"; }
count() { sid=$(recorded); [ -z "$sid" ] && { echo "  session not opened"; return; }
  api GET "/sessions/$sid/messages?limit=500" | python3 -c "
import json,sys; d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
print(' ', len(items), 'turn(s) |', (items[0]['body'] if items else ''), '→', (items[-1]['body'] if items else ''))"; }
sync_now > /dev/null; count
echo "== 5. the same transcript again: nothing is recorded twice"
sync_now > /dev/null; count

echo "== 6. archiving off: the project keeps only what a decision cites"
api PATCH "/projects/$PID" '[{"set_archive_transcripts":false}]' > /dev/null
cat >> "$T" <<EOF
{"type":"user","sessionId":"$SID","cwd":"/repo","timestamp":"2026-09-20T10:05:00Z","message":{"content":"turn 121"}}
EOF
sync_now | python3 -c "
import json,sys
raw=sys.stdin.read().strip()
print('  the hook says:', (json.loads(raw).get('systemMessage') if raw else None) or 'nothing')"
count

echo "== 7. the unseen filter agrees with the block (after three reads, nothing)"
api GET "/decisions?unseen=true" | python3 -c "
import json,sys; d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
print('  unseen over REST:', [i['title'] for i in items])"
