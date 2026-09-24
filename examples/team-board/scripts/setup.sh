#!/usr/bin/env bash
# One-shot, re-runnable setup for the team-board example, run by
# `scripts/dev.ts` (or standalone: `bun run setup`) once the server is up:
#
#   1. create the superuser via `POST /api/setup` (the first-run install
#      token flow, `CB_SETUP_TOKEN` — see crates/server/src/routes/setup.rs)
#      if one doesn't already exist
#   2. turn on `settings.teams.enabled` (off by default — see
#      crates/server/src/app.rs)
#   3. generate schema.json (scripts/gen-schema.ts) and apply it with
#      `cratebase schema push`
#   4. regenerate TypeScript types (scripts/typegen.sh)
#
# Env: CRATEBASE_URL, CB_SETUP_TOKEN, SUPERUSER_EMAIL, SUPERUSER_PASSWORD,
# CRATEBASE_BIN, CB_DATA_DIR (see scripts/dev.ts for the defaults this is
# normally invoked with).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

CRATEBASE_URL="${CRATEBASE_URL:-http://localhost:8090}"
CB_SETUP_TOKEN="${CB_SETUP_TOKEN:-team-board-dev-setup-token}"
SUPERUSER_EMAIL="${SUPERUSER_EMAIL:-admin@example.com}"
SUPERUSER_PASSWORD="${SUPERUSER_PASSWORD:-changeme123}"
CRATEBASE_BIN="${CRATEBASE_BIN:-cratebase}"
CB_DATA_DIR="${CB_DATA_DIR:-./pb_data}"

for bin in curl jq bun "$CRATEBASE_BIN"; do
  command -v "$bin" >/dev/null 2>&1 || { echo "error: '$bin' is required but not found on PATH" >&2; exit 1; }
done

echo "==> Waiting for Cratebase at ${CRATEBASE_URL} ..."
for _ in $(seq 1 60); do
  curl -fsS -m 2 "${CRATEBASE_URL}/api/health" >/dev/null 2>&1 && { echo "==> Server is up."; break; }
  sleep 1
done

echo "==> Ensuring superuser ${SUPERUSER_EMAIL} exists ..."
NEEDS_SETUP="$(curl -fsS "${CRATEBASE_URL}/api/setup/status" | jq -r .needsSetup)"
if [ "$NEEDS_SETUP" = "true" ]; then
  curl -fsS -X POST "${CRATEBASE_URL}/api/setup" \
    -H "x-setup-token: ${CB_SETUP_TOKEN}" -H 'content-type: application/json' \
    -d "$(jq -nc --arg email "$SUPERUSER_EMAIL" --arg password "$SUPERUSER_PASSWORD" \
      '{email:$email,password:$password,passwordConfirm:$password}')" >/dev/null
  echo "==> Created superuser."
else
  echo "==> Superuser already exists, skipping."
fi

ADMIN_TOKEN="$(curl -fsS -X POST "${CRATEBASE_URL}/api/collections/_superusers/auth-with-password" \
  -H 'content-type: application/json' \
  -d "$(jq -nc --arg identity "$SUPERUSER_EMAIL" --arg password "$SUPERUSER_PASSWORD" '{identity:$identity,password:$password}')" \
  | jq -r .token)"
if [ -z "$ADMIN_TOKEN" ] || [ "$ADMIN_TOKEN" = "null" ]; then
  echo "error: failed to log in as ${SUPERUSER_EMAIL}" >&2
  exit 1
fi

echo "==> Enabling settings.teams.enabled ..."
curl -fsS -X PATCH "${CRATEBASE_URL}/api/settings" \
  -H "authorization: Bearer ${ADMIN_TOKEN}" -H 'content-type: application/json' \
  -d '{"teams":{"enabled":true}}' >/dev/null

echo "==> Generating schema.json ..."
bun run scripts/gen-schema.ts

echo "==> Pushing schema.json ..."
"$CRATEBASE_BIN" schema push schema.json --dir "$CB_DATA_DIR"

echo "==> Regenerating TypeScript types ..."
bash scripts/typegen.sh

echo
echo "==> team-board schema is ready."
