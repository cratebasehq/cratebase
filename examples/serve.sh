#!/usr/bin/env bash
# Serves every examples/* app correctly in one shot.
#
# Why this exists: each example's index.html imports the local SDK via
# `../../sdk/js/dist/index.js` (see its import map). That path only
# resolves if the static server's root is the *repo root*, not the
# example's own directory — `cd examples/todo && python3 -m http.server`
# 404s on the SDK import, which silently breaks the page (no JS runs, no
# error is visible in the UI: forms fall back to native GET submits and
# reload with your form data in the query string, and any "loading…"
# badge never updates).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

PORT="${PORT:-4173}"

if [ ! -f sdk/js/dist/index.js ]; then
  echo "==> sdk/js/dist/index.js missing, building the SDK once..."
  (cd sdk/js && npm install && npm run build)
fi

echo "==> Serving examples on http://localhost:${PORT}"
echo "    Todo (register/login/CRUD/realtime): http://localhost:${PORT}/examples/todo/"
echo "    Realtime chat:                       http://localhost:${PORT}/examples/realtime-chat/"
echo "    Realtime cursors:                    http://localhost:${PORT}/examples/realtime-cursors/"
echo
echo "    Make sure \`cargo run -p cratebase-server --bin cratebase -- serve\` is running (http://localhost:8090)."
echo

exec npx --yes serve . -l "${PORT}"
