#!/usr/bin/env bash
# End-to-end Cratebase benchmark.
#
# Builds a release Cratebase binary, boots it on a fresh data directory,
# provisions a superuser + `posts` collection, runs `bench.ts` against it,
# and stops it. Results land in benchmarks/results/cratebase.json.
#
# Usage:
#   benchmarks/run.sh [--skip-build] [bench.ts args...]
#
# Any extra args are forwarded to bench.ts (e.g. --concurrency=1,20,50,100).
set -euo pipefail
# Without this a failing curl deep in a pipeline just exits with its own
# status and prints nothing, which is a miserable way to debug a 40-step
# script.
trap 'status=$?; echo "run.sh: failed at line $LINENO with exit $status" >&2' ERR

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${BENCH_WORK_DIR:-/tmp/cb-bench}"
CB_PORT=8091
EMAIL=admin@bench.dev
PASS=benchpass123
SKIP_BUILD=0
EXTRA=()

for arg in "$@"; do
  case "$arg" in
    --skip-build) SKIP_BUILD=1 ;;
    *) EXTRA+=("$arg") ;;
  esac
done

if [ "$SKIP_BUILD" = 0 ]; then
  (cd "$ROOT" && cargo build --release -p cratebase-server)
fi
CB_BIN="$ROOT/target/release/cratebase"

# A server from the previous run can still be holding the port, which
# makes the new one die with "Address already in use" while the benchmark
# happily measures the old process against a data directory that has
# since been deleted.
rm -rf "$WORK/cb-data"
mkdir -p "$WORK/cb-data" "$ROOT/benchmarks/results"

for _ in $(seq 1 40); do
  if ! ss -ltn 2>/dev/null | grep -qE ":$CB_PORT\b"; then break; fi
  echo "waiting for port $CB_PORT to free up" >&2
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

env "${CB_ENV[@]}" "$CB_BIN" superuser create "$EMAIL" "$PASS" >/dev/null

env "${CB_ENV[@]}" "$CB_BIN" serve >"$WORK/cb.log" 2>&1 &
CB_PID=$!
# `wait` so the port is actually free by the time this script exits;
# without it a second run races the first one's shutdown.
trap 'kill $CB_PID 2>/dev/null; wait $CB_PID 2>/dev/null; true' EXIT

wait_for() {
  for _ in $(seq 1 100); do
    if curl -sf "$1" >/dev/null; then return 0; fi
    sleep 0.1
  done
  echo "server at $1 did not come up" >&2
  exit 1
}
wait_for "http://127.0.0.1:$CB_PORT/api/health"

# --- provision the posts collection ----------------------------------------
token=$(curl -sf -X POST "http://127.0.0.1:$CB_PORT/api/collections/_superusers/auth-with-password" \
    -H 'content-type: application/json' \
    -d "{\"identity\":\"$EMAIL\",\"password\":\"$PASS\"}" |
    sed -E 's/.*"token":"([^"]+)".*/\1/')
if [ -z "$token" ]; then
    echo "superuser login failed" >&2
    exit 1
fi
curl -sf -X POST "http://127.0.0.1:$CB_PORT/api/collections" \
    -H "authorization: Bearer $token" -H 'content-type: application/json' \
    -d '{"name":"posts","type":"base",
         "fields":[{"name":"title","type":"text","required":true},
                   {"name":"content","type":"text"},
                   {"name":"published","type":"bool"}],
         "listRule":"","viewRule":"","createRule":"","updateRule":null,"deleteRule":null}' \
    >/dev/null || { echo "creating the posts collection failed" >&2; exit 1; }

# --- run ----------------------------------------------------------------------
cd "$ROOT"
bun run benchmarks/bench.ts --base-url="http://127.0.0.1:$CB_PORT" \
  --admin-email="$EMAIL" --admin-password="$PASS" \
  --label=Cratebase --out=benchmarks/results/cratebase.json "${EXTRA[@]}" >/dev/null

echo "wrote benchmarks/results/cratebase.json"
