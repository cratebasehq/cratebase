#!/usr/bin/env bash
# End-to-end Cratebase vs PocketBase benchmark.
#
# Builds a release Cratebase binary, boots both servers on fresh data
# directories, provisions an identical superuser + `posts` collection on
# each, runs `bench.ts` against both back to back, stops them, and prints
# a comparison table. Results land in benchmarks/results/*.json.
#
# Usage:
#   benchmarks/run.sh [--pb=/path/to/pocketbase] [--skip-build] [bench.ts args...]
#
# Any extra args are forwarded to bench.ts (e.g. --concurrency=1,20,50,100).
set -euo pipefail
# Without this a failing curl deep in a pipeline just exits with its own
# status and prints nothing, which is a miserable way to debug a 40-step
# script.
trap 'status=$?; echo "run.sh: failed at line $LINENO with exit $status" >&2' ERR

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PB_BIN="${PB_BIN:-/tmp/pb-bench/pb/pocketbase}"
PB_VERSION="0.40.2"
WORK="${BENCH_WORK_DIR:-/tmp/pb-bench}"
CB_PORT=8091
PB_PORT=8092
EMAIL=admin@bench.dev
PASS=benchpass123
SKIP_BUILD=0
EXTRA=()

for arg in "$@"; do
  case "$arg" in
    --pb=*) PB_BIN="${arg#--pb=}" ;;
    --skip-build) SKIP_BUILD=1 ;;
    *) EXTRA+=("$arg") ;;
  esac
done

if [ ! -x "$PB_BIN" ]; then
  echo "downloading PocketBase v$PB_VERSION to $PB_BIN"
  mkdir -p "$(dirname "$PB_BIN")"
  curl -sL -o "$WORK/pocketbase.zip" \
    "https://github.com/pocketbase/pocketbase/releases/download/v$PB_VERSION/pocketbase_${PB_VERSION}_linux_amd64.zip"
  unzip -o -q "$WORK/pocketbase.zip" -d "$(dirname "$PB_BIN")"
fi

if [ "$SKIP_BUILD" = 0 ]; then
  (cd "$ROOT" && cargo build --release -p cratebase-server)
fi
CB_BIN="$ROOT/target/release/cratebase"

# Two things make a second run fail where the first one worked, and both
# look like a Cratebase bug in the log rather than what they are:
#
#  * PocketBase writes an auto-migration for every collection created
#    through its API into `$WORK/pb_migrations` and replays it on the next
#    boot, so provisioning `posts` a second time is a name conflict.
#  * a server from the previous run can still be holding 8091/8092, which
#    makes the new one die with "Address already in use" while the
#    benchmark happily measures the old process against a data directory
#    that has since been deleted.
rm -rf "$WORK/cb-data" "$WORK/pb-data" "$WORK/pb_migrations"
mkdir -p "$WORK/cb-data" "$WORK/pb-data" "$ROOT/benchmarks/results"

for _ in $(seq 1 40); do
  if ! ss -ltn 2>/dev/null | grep -qE ":($CB_PORT|$PB_PORT)\b"; then break; fi
  echo "waiting for ports $CB_PORT/$PB_PORT to free up" >&2
  sleep 0.5
done

# Kept as an array rather than a wrapper function so the server can be
# started with `env "${CB_ENV[@]}" ... &`. Backgrounding a shell function
# instead makes `$!` the PID of the *subshell*, so the EXIT trap kills the
# subshell and leaves an orphaned server holding the port and, worse,
# available to be measured by the next run.
CB_ENV=(
  DATABASE_URL="sqlite://$WORK/cb-data/cratebase.db"
  CRATEBASE_DATA_DIR="$WORK/cb-data"
  STORAGE_LOCAL_DIR="$WORK/cb-data/storage"
  PORT="$CB_PORT"
  AUTH_RATE_LIMIT_ENABLED=false
  RUST_LOG=warn
)
cb_env() {
  env "${CB_ENV[@]}" "$@"
}

cb_env "$CB_BIN" superuser create "$EMAIL" "$PASS" >/dev/null
"$PB_BIN" superuser create "$EMAIL" "$PASS" --dir "$WORK/pb-data" >/dev/null

env "${CB_ENV[@]}" "$CB_BIN" serve >"$WORK/cb.log" 2>&1 &
CB_PID=$!
"$PB_BIN" serve --http="127.0.0.1:$PB_PORT" --dir="$WORK/pb-data" >"$WORK/pb.log" 2>&1 &
PB_PID=$!
# `wait` so the ports are actually free by the time this script exits;
# without it a second run races the first one's shutdown.
trap 'kill $CB_PID $PB_PID 2>/dev/null; wait $CB_PID $PB_PID 2>/dev/null; true' EXIT

wait_for() {
  for _ in $(seq 1 100); do
    if curl -sf "$1" >/dev/null; then return 0; fi
    sleep 0.1
  done
  echo "server at $1 did not come up" >&2
  exit 1
}
wait_for "http://127.0.0.1:$CB_PORT/api/health"
wait_for "http://127.0.0.1:$PB_PORT/api/health"

# --- provision the posts collection on each ---------------------------------
# Both servers speak the same API now, so this is one code path rather than
# two: superusers are an ordinary auth collection, and fields are `fields`.
provision() {
    local port="$1" label="$2"
    local token
    token=$(curl -sf -X POST "http://127.0.0.1:$port/api/collections/_superusers/auth-with-password" \
        -H 'content-type: application/json' \
        -d "{\"identity\":\"$EMAIL\",\"password\":\"$PASS\"}" |
        sed -E 's/.*"token":"([^"]+)".*/\1/')
    if [ -z "$token" ]; then
        echo "$label: superuser login failed" >&2
        exit 1
    fi
    curl -sf -X POST "http://127.0.0.1:$port/api/collections" \
        -H "authorization: Bearer $token" -H 'content-type: application/json' \
        -d '{"name":"posts","type":"base",
             "fields":[{"name":"title","type":"text","required":true},
                       {"name":"content","type":"text"},
                       {"name":"published","type":"bool"}],
             "listRule":"","viewRule":"","createRule":"","updateRule":null,"deleteRule":null}' \
        >/dev/null || { echo "$label: creating the posts collection failed" >&2; exit 1; }
}
provision "$CB_PORT" Cratebase
provision "$PB_PORT" PocketBase

# --- run ----------------------------------------------------------------------
cd "$ROOT"
bun run benchmarks/bench.ts --base-url="http://127.0.0.1:$CB_PORT" \
  --admin-email="$EMAIL" --admin-password="$PASS" \
  --label=Cratebase --out=benchmarks/results/cratebase.json "${EXTRA[@]}" >/dev/null
bun run benchmarks/bench.ts --base-url="http://127.0.0.1:$PB_PORT" \
  --admin-email="$EMAIL" --admin-password="$PASS" \
  --label=PocketBase --out=benchmarks/results/pocketbase.json "${EXTRA[@]}" >/dev/null

bun run benchmarks/compare.ts benchmarks/results/cratebase.json benchmarks/results/pocketbase.json
