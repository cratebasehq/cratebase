#!/usr/bin/env sh
# Create the film's demo schema (places / posts, on top of the built-in
# users collection) and seed it with a handful of real-looking records,
# against an already-running `cratebase dev` instance.
#
# Usage:
#   cratebase dev --dir /tmp/cb-film-demo   # note the printed superuser
#   CRATEBASE_SUPERUSER_EMAIL=admin@localhost \
#   CRATEBASE_SUPERUSER_PASSWORD=<printed password> \
#   marketing/video/capture/seed.sh
#
# Safe to re-run: collection creation is skipped (not failed) if the name
# already exists, and record creation is best-effort (a handful of
# "already exists"-shaped errors on a second run are expected and ignored).
#
# Field-shape gotchas this script works around, discovered while building
# it (the docs at site/src/content/docs/docs/reference/rest-api/collections.mdx
# are stale on both points — real source of truth is crates/core/src/field.rs
# and the server's own response shape):
#   - POST/PATCH /api/collections takes "fields", not "schema".
#   - A relation field's target is a flat "collectionId" key on the field
#     object (the real collection ID, e.g. "_pb_users_auth_" for `users`
#     — not its name), not a nested "options" object.

set -eu

BASE_URL="${CRATEBASE_URL:-http://127.0.0.1:8090}"
EMAIL="${CRATEBASE_SUPERUSER_EMAIL:?Set CRATEBASE_SUPERUSER_EMAIL (printed once by \`cratebase dev\`)}"
PASSWORD="${CRATEBASE_SUPERUSER_PASSWORD:?Set CRATEBASE_SUPERUSER_PASSWORD (printed once by \`cratebase dev\`)}"

json_get() { python3 -c "import sys,json;d=json.load(sys.stdin);print(d$1)"; }

TOKEN=$(curl -sS -X POST "$BASE_URL/api/collections/_superusers/auth-with-password" \
  -H "Content-Type: application/json" \
  -d "{\"identity\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" | json_get "['token']")

USERS_ID=$(curl -sS "$BASE_URL/api/collections/users" -H "Authorization: Bearer $TOKEN" | json_get "['id']")

echo "== places collection =="
curl -sS -X POST "$BASE_URL/api/collections" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{
  "name": "places",
  "type": "base",
  "fields": [
    { "id": "text_place_name", "name": "name", "type": "text", "required": true, "searchable": true },
    { "id": "text_place_desc", "name": "description", "type": "text", "searchable": true },
    { "id": "geo_place_loc", "name": "location", "type": "geoPoint" },
    { "id": "rel_place_owner", "name": "owner", "type": "relation", "collectionId": "'"$USERS_ID"'", "maxSelect": 1, "minSelect": 0, "cascadeDelete": false }
  ],
  "listRule": "", "viewRule": "",
  "createRule": "@request.auth.id != \"\"",
  "updateRule": "owner = @request.auth.id",
  "deleteRule": "owner = @request.auth.id"
}' > /dev/null || echo "  (already exists — skipping)"

echo "== posts collection =="
curl -sS -X POST "$BASE_URL/api/collections" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{
  "name": "posts",
  "type": "base",
  "fields": [
    { "id": "text_post_title", "name": "title", "type": "text", "required": true, "searchable": true },
    { "id": "text_post_body", "name": "body", "type": "editor", "searchable": true },
    { "id": "bool_post_published", "name": "published", "type": "bool" },
    { "id": "rel_post_author", "name": "author", "type": "relation", "collectionId": "'"$USERS_ID"'", "maxSelect": 1, "minSelect": 0, "cascadeDelete": false }
  ],
  "listRule": "published = true || author = @request.auth.id",
  "viewRule": "published = true || author = @request.auth.id",
  "createRule": "@request.auth.id != \"\"",
  "updateRule": "author = @request.auth.id",
  "deleteRule": "author = @request.auth.id"
}' > /dev/null || echo "  (already exists — skipping)"

echo "== demo users =="
NICO=$(curl -sS -X POST "$BASE_URL/api/collections/users/records" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"email":"nico@cratebase.dev","password":"DemoPass123!","passwordConfirm":"DemoPass123!","name":"Nico Audy","emailVisibility":true,"verified":true}' \
  | json_get "['id']" 2>/dev/null || curl -sS "$BASE_URL/api/collections/users/records?filter=email='nico@cratebase.dev'" -H "Authorization: Bearer $TOKEN" | json_get "['items'][0]['id']")
AMARA=$(curl -sS -X POST "$BASE_URL/api/collections/users/records" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"email":"amara@cratebase.dev","password":"DemoPass123!","passwordConfirm":"DemoPass123!","name":"Amara Chen","emailVisibility":true,"verified":true}' \
  | json_get "['id']" 2>/dev/null || curl -sS "$BASE_URL/api/collections/users/records?filter=email='amara@cratebase.dev'" -H "Authorization: Bearer $TOKEN" | json_get "['items'][0]['id']")
echo "  nico=$NICO amara=$AMARA"

echo "== places records =="
curl -sS -X POST "$BASE_URL/api/collections/places/records" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"name\":\"Kopi Kenangan Senopati\",\"description\":\"Third-wave coffee near the office, quiet before 9am.\",\"location\":{\"lon\":106.7996,\"lat\":-6.2407},\"owner\":\"$NICO\"}" > /dev/null
curl -sS -X POST "$BASE_URL/api/collections/places/records" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"name\":\"Warung Tekko\",\"description\":\"Grilled fish and sambal matah, worth a detour.\",\"location\":{\"lon\":106.8090,\"lat\":-6.2297},\"owner\":\"$AMARA\"}" > /dev/null
curl -sS -X POST "$BASE_URL/api/collections/places/records" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"name\":\"Blok M Co-working Loft\",\"description\":\"Fast wifi, standing desks, a rooftop for calls.\",\"location\":{\"lon\":106.8022,\"lat\":-6.2440},\"owner\":\"$NICO\"}" > /dev/null
curl -sS -X POST "$BASE_URL/api/collections/places/records" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"name\":\"Pasar Santa Vinyl Corner\",\"description\":\"A crate of second-hand vinyl worth digging through.\",\"location\":{\"lon\":106.8107,\"lat\":-6.2412},\"owner\":\"$AMARA\"}" > /dev/null

echo "== posts records =="
curl -sS -X POST "$BASE_URL/api/collections/posts/records" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"title\":\"Why we moved off three services onto one binary\",\"body\":\"<p>Collections, auth and realtime used to be three vendors. Now it is one crate.</p>\",\"published\":true,\"author\":\"$NICO\"}" > /dev/null
curl -sS -X POST "$BASE_URL/api/collections/posts/records" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"title\":\"Shipping the places search feature\",\"body\":\"<p>Full-text search across name and description, ranked, in one query parameter.</p>\",\"published\":true,\"author\":\"$AMARA\"}" > /dev/null
curl -sS -X POST "$BASE_URL/api/collections/posts/records" -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"title\":\"Draft: nearest-place notes\",\"body\":\"<p>PostGIS acceleration notes, not published yet.</p>\",\"published\":false,\"author\":\"$NICO\"}" > /dev/null

echo "Done. places/posts seeded, ready for capture/capture.mjs."
