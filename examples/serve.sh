#!/usr/bin/env bash
# Serves every examples/* app correctly in one shot.
#
# Todo/realtime-chat/realtime-cursors/docmind are static HTML/JS apps that
# import `@cratebase/client` through an import map (see each index.html's
# `<script type="importmap">`). That map points at
# examples/lib/cratebase-client.js, a single-file browser ESM bundle of
# sdk/js/client that this script builds first (it is gitignored). A static
# file server then serves everything. It's rooted at the repo root (not
# each example's own directory) so one process serves all of them and the
# import map's absolute `/examples/lib/...` path resolves.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

PORT="${PORT:-4173}"

echo "==> Bundling @cratebase/client for the browser -> examples/lib/cratebase-client.js"
bun build sdk/js/client/src/index.ts \
  --target=browser --format=esm \
  --outfile=examples/lib/cratebase-client.js

echo "==> Serving examples on http://localhost:${PORT}"
echo "    Todo (register/login/CRUD/realtime): http://localhost:${PORT}/examples/todo/"
echo "    Realtime chat:                       http://localhost:${PORT}/examples/realtime-chat/"
echo "    Realtime cursors:                    http://localhost:${PORT}/examples/realtime-cursors/"
echo "    DocMind (needs examples/docmind/setup.sh):  http://localhost:${PORT}/examples/docmind/"
echo "    Kanban (own Vite dev server, not this one): bun --cwd examples/kanban install && bun --cwd examples/kanban run dev"
echo
echo "    Make sure \`cargo run -p cratebase-server --bin cratebase -- serve\` is running (http://localhost:8090)."
echo

exec npx --yes serve . -l "${PORT}"
