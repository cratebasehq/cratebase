#!/usr/bin/env bash
# Serves every examples/* app correctly in one shot.
#
# Todo/realtime-chat/realtime-cursors are static HTML/JS apps that load
# the published `pocketbase` SDK straight from esm.sh via an import map
# (see each index.html's `<script type="importmap">`) — there is no
# local SDK build to produce first, so this just needs a static file
# server. It's rooted at the repo root (not each example's own
# directory) only so the same one process can serve all three examples
# at once; none of them actually reach outside their own folder for
# anything else.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

PORT="${PORT:-4173}"

echo "==> Serving examples on http://localhost:${PORT}"
echo "    Todo (register/login/CRUD/realtime): http://localhost:${PORT}/examples/todo/"
echo "    Realtime chat:                       http://localhost:${PORT}/examples/realtime-chat/"
echo "    Realtime cursors:                    http://localhost:${PORT}/examples/realtime-cursors/"
echo "    Kanban (own Vite dev server, not this one): npm --prefix examples/kanban install && npm --prefix examples/kanban run dev"
echo
echo "    Make sure \`cargo run -p cratebase-server --bin cratebase -- serve\` is running (http://localhost:8090)."
echo

exec npx --yes serve . -l "${PORT}"
