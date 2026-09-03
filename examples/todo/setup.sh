#!/usr/bin/env bash
# One-shot, re-runnable setup for the todo example: ensures a superuser
# exists and creates (or updates) the `todos` collection, gated to
# signed-in users only — the example itself handles registering/signing
# in end users via the default `users` auth collection, no setup needed
# for that part.
#
# Usage: examples/todo/setup.sh
# Env: CRATEBASE_URL, ADMIN_EMAIL, ADMIN_PASSWORD (see setup-common.sh)
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
source ../lib/setup-common.sh

wait_for_server
ensure_superuser_and_login
ensure_collection "todos" '{
  "name": "todos",
  "type": "base",
  "schema": [
    { "id": "title", "name": "title", "type": "text", "required": true },
    { "id": "done", "name": "done", "type": "bool", "required": false }
  ],
  "listRule": "@request.auth.id != \"\"",
  "viewRule": "@request.auth.id != \"\"",
  "createRule": "@request.auth.id != \"\"",
  "updateRule": "@request.auth.id != \"\"",
  "deleteRule": "@request.auth.id != \"\""
}'

echo
echo "==> Todo example is ready. Run examples/serve.sh and open:"
echo "    http://localhost:4173/examples/todo/"
