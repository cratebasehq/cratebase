#!/usr/bin/env bash
# Shared helpers for examples/*/setup.sh. Not meant to be run directly.
#
# Env vars every setup.sh honors:
#   CRATEBASE_URL   default http://localhost:8090
#   ADMIN_EMAIL     default admin@example.com
#   ADMIN_PASSWORD  default changeme123 (min 8 chars — Cratebase requires it)
set -euo pipefail

CRATEBASE_URL="${CRATEBASE_URL:-http://localhost:8090}"
ADMIN_EMAIL="${ADMIN_EMAIL:-admin@example.com}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-changeme123}"

require_bin() {
  for bin in "$@"; do
    command -v "$bin" >/dev/null 2>&1 || {
      echo "error: '$bin' is required but not found on PATH" >&2
      exit 1
    }
  done
}

wait_for_server() {
  echo "==> Waiting for Cratebase at ${CRATEBASE_URL} ..."
  for _ in $(seq 1 30); do
    if curl -fsS -m 2 "${CRATEBASE_URL}/api/health" >/dev/null 2>&1; then
      echo "==> Server is up."
      return 0
    fi
    sleep 1
  done
  echo "error: ${CRATEBASE_URL} isn't responding. Start it first:" >&2
  echo "  cargo run -p cratebase-server --bin cratebase -- serve" >&2
  exit 1
}

# Creates the superuser if missing, or resets its password if it already
# exists — safe to re-run. Prints nothing; sets ADMIN_TOKEN.
ensure_superuser_and_login() {
  require_bin cargo curl jq
  local repo_root
  repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  echo "==> Ensuring superuser ${ADMIN_EMAIL} exists ..."
  (cd "$repo_root" && cargo run -q -p cratebase-server --bin cratebase -- \
    superuser upsert "$ADMIN_EMAIL" "$ADMIN_PASSWORD" >/dev/null)

  ADMIN_TOKEN="$(curl -fsS -X POST "${CRATEBASE_URL}/api/admins/auth-with-password" \
    -H 'content-type: application/json' \
    -d "$(jq -nc --arg email "$ADMIN_EMAIL" --arg password "$ADMIN_PASSWORD" '{email:$email,password:$password}')" \
    | jq -r .token)"

  if [ -z "$ADMIN_TOKEN" ] || [ "$ADMIN_TOKEN" = "null" ]; then
    echo "error: failed to log in as ${ADMIN_EMAIL}" >&2
    exit 1
  fi
  export ADMIN_TOKEN
}

# Args: $1 = collection name, $2 = JSON body for POST/PATCH /api/collections.
# Creates the collection if missing; PATCHes it to match `body` if it
# already exists (so a stale schema/rules from an older run of this
# script — or a manual edit — gets brought back in line). Idempotent.
ensure_collection() {
  local name="$1" body="$2"
  local status
  status="$(curl -s -o /dev/null -w '%{http_code}' "${CRATEBASE_URL}/api/collections/${name}" \
    -H "authorization: Bearer ${ADMIN_TOKEN}")"

  local method=POST url="${CRATEBASE_URL}/api/collections" verb=Created gerund=Creating
  if [ "$status" = "200" ]; then
    method=PATCH
    url="${CRATEBASE_URL}/api/collections/${name}"
    verb=Updated
    gerund=Updating
  fi

  echo "==> ${gerund} collection '${name}' ..."
  local http_code
  http_code="$(curl -sS -o /tmp/cratebase-setup-resp.$$ -w '%{http_code}' -X "$method" "$url" \
    -H "authorization: Bearer ${ADMIN_TOKEN}" -H 'content-type: application/json' \
    -d "$body")"
  if [ "$http_code" -ge 300 ]; then
    echo "error: failed to ${verb,,} collection '${name}' (HTTP ${http_code}):" >&2
    cat /tmp/cratebase-setup-resp.$$ >&2
    rm -f /tmp/cratebase-setup-resp.$$
    exit 1
  fi
  rm -f /tmp/cratebase-setup-resp.$$
  echo "==> ${verb} '${name}'."
}
