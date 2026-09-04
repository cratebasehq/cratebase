#!/usr/bin/env bash
# One-shot, re-runnable setup for the realtime cursors example: ensures a
# superuser exists and creates the `cursors` collection if missing.
#
# Usage: examples/realtime-cursors/setup.sh
# Env: CRATEBASE_URL, ADMIN_EMAIL, ADMIN_PASSWORD (see setup-common.sh)
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
source ../lib/setup-common.sh

wait_for_server
ensure_superuser_and_login
ensure_collection "cursors" '{
  "name": "cursors",
  "type": "base",
  "fields": [
    { "name": "clientId", "type": "text", "required": true },
    { "name": "x", "type": "number", "required": true },
    { "name": "y", "type": "number", "required": true },
    { "name": "color", "type": "text", "required": true },
    { "name": "label", "type": "text" },
    { "name": "created", "type": "autodate", "onCreate": true },
    { "name": "updated", "type": "autodate", "onCreate": true, "onUpdate": true }
  ],
  "indexes": [
    "CREATE UNIQUE INDEX `idx_cursors_clientId` ON `cursors` (`clientId`)"
  ],
  "listRule": "",
  "viewRule": "",
  "createRule": "",
  "updateRule": "",
  "deleteRule": ""
}'

echo
echo "==> Realtime cursors example is ready. Run examples/serve.sh and open:"
echo "    http://localhost:4173/examples/realtime-cursors/"
