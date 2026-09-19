#!/bin/sh
# Smoke of `converge hook poll` against a running server: a session
# hears the signals that touch the project its working tree is bound
# to, and nothing else. A claim consumes, so a signal handed to the
# wrong session is one the right session never hears — this is the
# check that it does not happen.
#
#   cargo xtask dev
#   scripts/smoke-poll.sh http://127.0.0.1:8080 <token>
set -eu
BASE=${1:?server url}; TOKEN=${2:?bearer token}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cargo build -q -p converge-cli --manifest-path "$ROOT/Cargo.toml"
CONVERGE="$ROOT/target/debug/converge"
S=$(mktemp -d); trap 'rm -rf "$S"' EXIT
RUN="$(date +%s)-$$"   # sessions are keyed by external id, globally

id() { python3 -c "import json,sys; print(json.load(sys.stdin)['id'])"; }
api() { m=$1; p=$2; b=${3:-}; if [ -n "$b" ]; then curl -s -X "$m" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d "$b" "$BASE/api/v1$p"; else curl -s -X "$m" -H "Authorization: Bearer $TOKEN" "$BASE/api/v1$p"; fi; }
ME=$(api GET /users/me | python3 -c "import json,sys; print(json.load(sys.stdin)['id'])")

# One group, two projects: both visible to the same person, so
# visibility is not what keeps them apart.
GID=$(api POST /groups '{"name":"poll","kind":"shared"}' | id)
HERE=$(api POST /projects "{\"group_id\":\"$GID\",\"name\":\"here-$RUN\"}" | id)
THERE=$(api POST /projects "{\"group_id\":\"$GID\",\"name\":\"there-$RUN\"}" | id)
decision() { api POST /decisions "{\"project_id\":\"$1\",\"status\":\"accepted\",\"title\":\"$2\",\"summary\":\"s\",\"context\":null,\"consequences\":null}" | id; }
D_HERE=$(decision "$HERE" "ours")
D_THERE=$(decision "$THERE" "theirs")
D_OTHER=$(decision "$THERE" "also theirs")

# A working tree bound to `here`, and its own state directory.
mkdir -p "$S/home/.config/converge" "$S/state" "$S/here"
printf 'server = "%s"\ntoken = "%s"\n' "$BASE" "$TOKEN" > "$S/home/.config/converge/cli.toml"
printf 'project_id = "%s"\n' "$HERE" > "$S/here/.converge"
export HOME="$S/home" XDG_CONFIG_HOME="$S/home/.config" XDG_STATE_HOME="$S/state"

SID="poll-$RUN"
poll() { printf '{"cwd":"%s","session_id":"%s","hook_event_name":"UserPromptSubmit"}' "$S/here" "$SID" | "$CONVERGE" hook poll --harness claude; }
# The poll runs at most once every 20 s; the smoke moves its clock back.
lift() { python3 -c "
import json; p='$XDG_STATE_HOME/converge/poll.json'
d=json.load(open(p)); d['$SID']['at']=0; json.dump(d,open(p,'w'))" 2>/dev/null || true; }
show() { python3 -c "
import json,sys
raw=sys.stdin.read().strip()
if not raw: print('  nothing'); raise SystemExit
d=json.loads(raw); ctx=(d.get('hookSpecificOutput') or {}).get('additionalContext') or ''
titles=[l.strip('- ').split(' (')[0] for l in ctx.splitlines() if l.startswith('- ')]
print('  delivered:', titles or 'nothing')"; }
signal() { api POST /signals "{\"source\":\"$1\",\"targets\":[\"$2\"],\"kind\":\"$3\",\"tier\":\"conflict\",\"title\":\"$4\",\"text\":\"t\",\"consequence\":null,\"recommendation\":null,\"produced_by\":{\"user\":\"$ME\"}}" > /dev/null; }

echo "== 1. first poll: the session opens its row and is handed nothing"
poll | show; lift

echo "== 2. a signal wholly inside the other project"
signal "$D_THERE" "$D_OTHER" "duplication" "next door only"
poll | show; lift

echo "== 3. a signal reaching into this one"
signal "$D_THERE" "$D_HERE" "divergence" "reaches us"
poll | show; lift

echo "== 4. and the one it never heard is still unclaimed for whoever it belongs to"
api GET "/signals?project=$THERE&unseen=true" | python3 -c "
import json,sys
d=json.load(sys.stdin); items=d['items'] if isinstance(d,dict) else d
print('  still new over there:', [i['title'] for i in items])"
