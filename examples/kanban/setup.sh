#!/usr/bin/env bash
# One-shot, re-runnable setup for the kanban example: ensures a superuser
# exists and creates (or updates) the `cards` and `presence` collections.
# The example itself handles registering/signing in end users via the
# default `users` auth collection, no setup needed for that part.
#
# Usage: examples/kanban/setup.sh
# Env: CRATEBASE_URL, ADMIN_EMAIL, ADMIN_PASSWORD (see setup-common.sh)
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
source ../lib/setup-common.sh

wait_for_server
ensure_superuser_and_login
ensure_collection "cards" '{
  "name": "cards",
  "type": "base",
  "fields": [
    { "name": "title", "type": "text", "required": true },
    { "name": "status", "type": "select", "required": true, "maxSelect": 1, "values": ["todo", "in_progress", "done"] },
    { "name": "order", "type": "number", "required": true },
    { "name": "created", "type": "autodate", "onCreate": true },
    { "name": "updated", "type": "autodate", "onCreate": true, "onUpdate": true }
  ],
  "listRule": "@request.auth.id != \"\"",
  "viewRule": "@request.auth.id != \"\"",
  "createRule": "@request.auth.id != \"\"",
  "updateRule": "@request.auth.id != \"\"",
  "deleteRule": "@request.auth.id != \"\""
}'
ensure_collection "presence" '{
  "name": "presence",
  "type": "base",
  "fields": [
    { "name": "userId", "type": "text", "required": true },
    { "name": "name", "type": "text", "required": true },
    { "name": "lastSeen", "type": "autodate", "onCreate": true, "onUpdate": true }
  ],
  "indexes": [
    "CREATE UNIQUE INDEX `idx_presence_userId` ON `presence` (`userId`)"
  ],
  "listRule": "@request.auth.id != \"\"",
  "viewRule": "@request.auth.id != \"\"",
  "createRule": "@request.auth.id != \"\"",
  "updateRule": "@request.auth.id != \"\"",
  "deleteRule": "@request.auth.id != \"\""
}'

echo
echo "==> Kanban example is ready."
echo "    cd examples/kanban && npm install && npm run dev"
echo "    then open the printed http://localhost:5174/ URL."
