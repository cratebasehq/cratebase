#!/usr/bin/env bash
# One-shot, re-runnable setup for the realtime chat example: ensures a
# superuser exists and creates the `messages` collection if missing.
#
# Usage: examples/realtime-chat/setup.sh
# Env: CRATEBASE_URL, ADMIN_EMAIL, ADMIN_PASSWORD (see setup-common.sh)
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
source ../lib/setup-common.sh

wait_for_server
ensure_superuser_and_login
ensure_collection "messages" '{
  "name": "messages",
  "type": "base",
  "fields": [
    { "name": "author", "type": "text", "required": true },
    { "name": "content", "type": "text", "required": true },
    { "name": "created", "type": "autodate", "onCreate": true },
    { "name": "updated", "type": "autodate", "onCreate": true, "onUpdate": true }
  ],
  "listRule": "",
  "viewRule": "",
  "createRule": "",
  "updateRule": null,
  "deleteRule": null
}'

echo
echo "==> Realtime chat example is ready. Run examples/serve.sh and open:"
echo "    http://localhost:4173/examples/realtime-chat/"
