#!/usr/bin/env bash
# One-shot, re-runnable setup for the team-board example, run by
# `scripts/dev.ts` (or standalone: `bun run setup`) once the server is up:
#
#   1. create the superuser via `POST /api/setup` (the first-run install
#      token flow, `CB_SETUP_TOKEN` — see crates/server/src/routes/setup.rs)
#      if one doesn't already exist
#   2. turn on `settings.teams.enabled` (off by default — see
#      crates/server/src/app.rs)
#   3. generate schema.json (scripts/gen-schema.ts) and apply it via
#      `POST /api/schema/apply` (deliberately *not* the `cratebase schema
#      push` CLI — see "Known Cratebase issue" below)
#   4. regenerate TypeScript types via `GET /api/typegen` (same reasoning)
#
# Env: CRATEBASE_URL, CB_SETUP_TOKEN, SUPERUSER_EMAIL, SUPERUSER_PASSWORD
# (see scripts/dev.ts for the defaults this is normally invoked with).
#
# Known Cratebase issue this works around: `cratebase schema push`/
# `cratebase typegen` are CLI subcommands that open their own `App`
# against the data directory directly (`crates/server/src/main.rs`'s
# `schema()`/`typegen()`), bypassing whatever `cratebase serve` process
# already has that directory open. Live-verified while building this
# example: running `cratebase schema push schema.json --dir ./pb_data`
# against a directory a `cratebase serve --dev` process already had open
# updated the on-disk database (confirmed via a fresh `GET
# /api/collections/<name>` after restarting the server) but the *running*
# server kept 404ing every request against the newly created collections
# ("Missing collection context") until it was restarted — its in-memory
# collection registry never picked up the change. `POST /api/schema/apply`
# and `GET /api/typegen` run the exact same underlying logic
# (`routes::schema::plan_and_apply`, `typegen::generate` — see their doc
# comments) *inside* the already-running server process, so its own cache
# updates immediately and this whole class of bug doesn't apply. Prefer
# these HTTP endpoints over the CLI subcommands whenever a `serve` process
# for the same directory might already be running — which is exactly
# `bun run dev`'s situation.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

CRATEBASE_URL="${CRATEBASE_URL:-http://localhost:8090}"
CB_SETUP_TOKEN="${CB_SETUP_TOKEN:-team-board-dev-setup-token}"
SUPERUSER_EMAIL="${SUPERUSER_EMAIL:-admin@example.com}"
SUPERUSER_PASSWORD="${SUPERUSER_PASSWORD:-changeme123}"

for bin in curl jq bun; do
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

echo "==> Pushing schema.json (POST /api/schema/apply) ..."
curl -fsS -X POST "${CRATEBASE_URL}/api/schema/apply" \
  -H "authorization: Bearer ${ADMIN_TOKEN}" -H 'content-type: application/json' \
  --data-binary @schema.json | jq -r '.collections[] | "\(.action)\t\(.name)"'

echo "==> Regenerating TypeScript types (GET /api/typegen) ..."
curl -fsS "${CRATEBASE_URL}/api/typegen" -H "authorization: Bearer ${ADMIN_TOKEN}" -o src/cratebase-types.d.ts
echo "==> wrote types to src/cratebase-types.d.ts"

echo
echo "==> team-board schema is ready."
