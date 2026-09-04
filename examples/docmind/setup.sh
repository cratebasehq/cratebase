#!/usr/bin/env bash
# One-shot, re-runnable setup for the DocMind example: ensures a
# superuser exists and creates (or updates) the `docs`, `chunks`, and
# `messages` collections. Unlike every other example's setup.sh, two of
# these need a real collection id at creation time, not just a name:
# `chunks.docId` and `messages.author` are `relation` fields, and
# `FieldKind::Relation::collection_id` (`crates/core/src/field.rs`) must
# be an actual collection id — the API does not resolve a name for you.
# So `docs` is created first, its id is read back, and `chunks` is built
# with that id baked into its `docId` field's `collectionId` (same for
# `messages.author` against the built-in `users` collection).
#
# Usage: examples/docmind/setup.sh
# Env: CRATEBASE_URL, ADMIN_EMAIL, ADMIN_PASSWORD (see setup-common.sh)
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
source ../lib/setup-common.sh

collection_id() {
  curl -fsS "${CRATEBASE_URL}/api/collections/$1" -H "authorization: Bearer ${ADMIN_TOKEN}" | jq -r '.id'
}

wait_for_server
ensure_superuser_and_login

# pb_hooks/docmind.pb.js's `docs` hook and its `docmind_index` cron job
# both make self-`$http.send` calls against this server's own
# `meta.appURL` (to trigger the vector field's auto-embedding through
# the real record API — see that file's module doc for why). The
# PocketBase default, `http://localhost:8090`, only matches if the
# server really is bound there; point this at `CRATEBASE_URL` explicitly
# so the example works the same way against any host/port.
echo "==> Setting meta.appURL to ${CRATEBASE_URL} (pb_hooks self-calls depend on this) ..."
curl -fsS -X PATCH "${CRATEBASE_URL}/api/settings" \
  -H "authorization: Bearer ${ADMIN_TOKEN}" -H 'content-type: application/json' \
  -d "$(jq -nc --arg url "$CRATEBASE_URL" '{meta:{appURL:$url}}')" >/dev/null

ensure_collection "docs" '{
  "name": "docs",
  "type": "base",
  "fields": [
    { "name": "title", "type": "text", "required": true },
    { "name": "file", "type": "file", "maxSelect": 1, "maxSize": 10485760 },
    { "name": "created", "type": "autodate", "onCreate": true },
    { "name": "updated", "type": "autodate", "onCreate": true, "onUpdate": true }
  ],
  "listRule": "@request.auth.id != \"\"",
  "viewRule": "@request.auth.id != \"\"",
  "createRule": "@request.auth.id != \"\"",
  "updateRule": null,
  "deleteRule": null
}'

DOCS_ID="$(collection_id docs)"
USERS_ID="$(collection_id users)"
echo "==> docs collection id:  ${DOCS_ID}"
echo "==> users collection id: ${USERS_ID}"

# `chunks` is written exclusively by pb_hooks/docmind.pb.js (superuser
# token, real HTTP call so the vector field's auto-embedding runs) — no
# end user or app code creates a row here directly, hence createRule:
# null. `docId` is optional (minSelect 0): the hook's own retrieval path
# round-trips a query through this same collection with no docId set
# (see docmind.pb.js's "Why a throwaway record" comment).
ensure_collection "chunks" "$(cat <<JSON
{
  "name": "chunks",
  "type": "base",
  "fields": [
    { "name": "docId", "type": "relation", "collectionId": "${DOCS_ID}", "cascadeDelete": true, "minSelect": 0, "maxSelect": 1 },
    { "name": "text", "type": "text", "required": true },
    { "name": "chunkIndex", "type": "number" },
    { "name": "embedding", "type": "vector", "dimensions": 64, "embedding": { "provider": "echo", "model": "", "sourceField": "text" } },
    { "name": "created", "type": "autodate", "onCreate": true }
  ],
  "listRule": "@request.auth.id != \"\"",
  "viewRule": "@request.auth.id != \"\"",
  "createRule": null,
  "updateRule": null,
  "deleteRule": null
}
JSON
)"

# `author` is required (minSelect 1) and every rule pins it to
# `@request.auth.id` — this is the per-employee conversation scoping the
# spec's DocMind story calls for (§8 item 2). See docmind.pb.js for why
# this collection is written by the hook itself rather than the LLM
# gateway's own optional `collection` persistence.
ensure_collection "messages" "$(cat <<JSON
{
  "name": "messages",
  "type": "base",
  "fields": [
    { "name": "author", "type": "relation", "collectionId": "${USERS_ID}", "cascadeDelete": true, "minSelect": 1, "maxSelect": 1 },
    { "name": "prompt", "type": "text", "required": true },
    { "name": "response", "type": "text", "required": true },
    { "name": "model", "type": "text" },
    { "name": "created", "type": "autodate", "onCreate": true }
  ],
  "listRule": "@request.auth.id != \"\" && author = @request.auth.id",
  "viewRule": "@request.auth.id != \"\" && author = @request.auth.id",
  "createRule": "@request.auth.id != \"\" && author = @request.auth.id",
  "updateRule": null,
  "deleteRule": null
}
JSON
)"

echo
echo "==> DocMind example is ready."
echo "    Restart the server (if it wasn't already), run from the repo"
echo "    root, with this example's pb_hooks/ enabled and DOCMIND_DATA_DIR"
echo "    pointed at the same data directory --dir/CRATEBASE_DATA_DIR uses"
echo "    (default ./pb_data — see README.md for why the hook needs a"
echo "    direct path to it):"
echo "      CB_HOOKS_DIR=\"\$PWD/examples/docmind/pb_hooks\" DOCMIND_DATA_DIR=\"\$PWD/pb_data\" \\"
echo "        cargo run -p cratebase-server --bin cratebase -- serve"
echo "    Then run 'bun run examples:serve' and open:"
echo "      http://localhost:4173/examples/docmind/"
