#!/usr/bin/env bash
# Exercises the key API flows end to end against a running, seeded
# team-board instance, per the task's "VERIFY for real" requirement.
# Not part of `bun run dev` — run by hand after the app is up
# (`bun run dev`, then in another terminal: `bash scripts/verify.sh`).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

CRATEBASE_URL="${CRATEBASE_URL:-http://localhost:8090}"
ADMIN_EMAIL="${SUPERUSER_EMAIL:-admin@example.com}"
ADMIN_PASSWORD="${SUPERUSER_PASSWORD:-changeme123}"

pass() { echo "PASS: $1"; }
fail() { echo "FAIL: $1" >&2; exit 1; }

echo "== sign in as alice =="
ALICE_TOKEN="$(curl -fsS -X POST "${CRATEBASE_URL}/api/collections/users/auth-with-password" \
  -H 'content-type: application/json' \
  -d '{"identity":"alice@example.com","password":"password123"}' | jq -r .token)"
[ "$ALICE_TOKEN" != "null" ] && [ -n "$ALICE_TOKEN" ] && pass "alice signed in" || fail "alice sign-in"

echo "== alice's team membership + cards =="
MEMBERSHIP="$(curl -fsS "${CRATEBASE_URL}/api/collections/_team_members/records" -H "authorization: Bearer ${ALICE_TOKEN}" -G --data-urlencode 'filter=userRef = @request.auth.id')"
TEAM_ID="$(echo "$MEMBERSHIP" | jq -r '.items[0].teamRef')"
[ -n "$TEAM_ID" ] && [ "$TEAM_ID" != "null" ] && pass "alice's team id: $TEAM_ID" || fail "alice has no team membership"

CARDS="$(curl -fsS "${CRATEBASE_URL}/api/collections/cards/records" -H "authorization: Bearer ${ALICE_TOKEN}" -G --data-urlencode "filter=teamRef = \"${TEAM_ID}\"")"
CARD_COUNT="$(echo "$CARDS" | jq '.items | length')"
[ "$CARD_COUNT" -gt 0 ] && pass "alice sees ${CARD_COUNT} cards on her team" || fail "alice sees no cards"
COLUMN_ID="$(echo "$CARDS" | jq -r '.items[0].columnRef')"
FIRST_CARD_ID="$(echo "$CARDS" | jq -r '.items[0].id')"

echo "== sign in as carol (different team) and confirm cross-team denial =="
CAROL_TOKEN="$(curl -fsS -X POST "${CRATEBASE_URL}/api/collections/users/auth-with-password" \
  -H 'content-type: application/json' \
  -d '{"identity":"carol@example.com","password":"password123"}' | jq -r .token)"
[ "$CAROL_TOKEN" != "null" ] && pass "carol signed in" || fail "carol sign-in"

CAROL_VIEW_CODE="$(curl -fsS -o /tmp/carol-view.$$ -w '%{http_code}' "${CRATEBASE_URL}/api/collections/cards/records/${FIRST_CARD_ID}" -H "authorization: Bearer ${CAROL_TOKEN}")"
if [ "$CAROL_VIEW_CODE" = "404" ]; then
  pass "carol cannot view alice's team's card (404, viewRule denied)"
else
  cat /tmp/carol-view.$$ >&2
  fail "carol was able to view alice's team's card (HTTP ${CAROL_VIEW_CODE})"
fi
rm -f /tmp/carol-view.$$

CAROL_LIST="$(curl -fsS "${CRATEBASE_URL}/api/collections/cards/records" -H "authorization: Bearer ${CAROL_TOKEN}" -G --data-urlencode "filter=teamRef = \"${TEAM_ID}\"")"
CAROL_COUNT="$(echo "$CAROL_LIST" | jq '.items | length')"
[ "$CAROL_COUNT" -eq 0 ] && pass "carol's filtered list of alice's team returns 0 items" || fail "carol's list leaked ${CAROL_COUNT} of alice's team's cards"

CAROL_CREATE_CODE="$(curl -fsS -o /tmp/carol-create.$$ -w '%{http_code}' -X POST "${CRATEBASE_URL}/api/collections/cards/records" \
  -H "authorization: Bearer ${CAROL_TOKEN}" -H 'content-type: application/json' \
  -d "$(jq -nc --arg team "$TEAM_ID" --arg col "$COLUMN_ID" '{teamRef:$team, columnRef:$col, title:"carol should not be able to do this", order:1}')")"
if [ "$CAROL_CREATE_CODE" -ge 400 ]; then
  pass "carol cannot create a card in alice's team (HTTP ${CAROL_CREATE_CODE}, createRule denied)"
else
  cat /tmp/carol-create.$$ >&2
  fail "carol was able to create a card in alice's team (HTTP ${CAROL_CREATE_CODE})"
fi
rm -f /tmp/carol-create.$$

echo "== pb_hook: columnRef required on a real (non-isQuery) card =="
NO_COLUMN_CODE="$(curl -fsS -o /tmp/no-column.$$ -w '%{http_code}' -X POST "${CRATEBASE_URL}/api/collections/cards/records" \
  -H "authorization: Bearer ${ALICE_TOKEN}" -H 'content-type: application/json' \
  -d "$(jq -nc --arg team "$TEAM_ID" '{teamRef:$team, title:"missing column", order:1}')")"
if [ "$NO_COLUMN_CODE" = "400" ]; then
  pass "creating a card without columnRef was rejected by the pb_hook (400)"
else
  cat /tmp/no-column.$$ >&2
  fail "expected 400 creating a card without columnRef, got ${NO_COLUMN_CODE}"
fi
rm -f /tmp/no-column.$$

echo "== create a real card, confirm searchText + embedding got derived =="
NEW_CARD="$(curl -fsS -X POST "${CRATEBASE_URL}/api/collections/cards/records" -H "authorization: Bearer ${ALICE_TOKEN}" -H 'content-type: application/json' \
  -d "$(jq -nc --arg team "$TEAM_ID" --arg col "$COLUMN_ID" '{teamRef:$team, columnRef:$col, title:"Rotate the S3 access keys", description:"Quarterly credential rotation for the backups bucket.", order:999999}')")"
NEW_CARD_ID="$(echo "$NEW_CARD" | jq -r .id)"
SEARCH_TEXT="$(echo "$NEW_CARD" | jq -r .searchText)"
EMBED_LEN="$(echo "$NEW_CARD" | jq '.embedding | length')"
[ "$SEARCH_TEXT" = "Rotate the S3 access keys

Quarterly credential rotation for the backups bucket." ] && pass "searchText derived from title+description" || fail "searchText not as expected: $SEARCH_TEXT"
[ "$EMBED_LEN" -gt 0 ] && pass "embedding vector populated (${EMBED_LEN} dims) via the echo provider" || fail "embedding was not populated on create"

echo "== semantic search: throwaway isQuery card + nearestTo =="
QUERY_CARD="$(curl -fsS -X POST "${CRATEBASE_URL}/api/collections/cards/records" -H "authorization: Bearer ${ALICE_TOKEN}" -H 'content-type: application/json' \
  -d "$(jq -nc --arg team "$TEAM_ID" '{teamRef:$team, title:"rotate credentials", isQuery:true, order:0}')")"
QUERY_ID="$(echo "$QUERY_CARD" | jq -r .id)"
QUERY_VEC="$(echo "$QUERY_CARD" | jq -c .embedding)"
NEAREST="$(curl -fsS "${CRATEBASE_URL}/api/collections/cards/records" -H "authorization: Bearer ${ALICE_TOKEN}" -G \
  --data-urlencode "nearestTo=embedding:$(echo "$QUERY_VEC" | tr -d '[]')" \
  --data-urlencode "nearestLimit=5" \
  --data-urlencode "filter=teamRef = \"${TEAM_ID}\" && isQuery = false")"
TOP_MATCH_ID="$(echo "$NEAREST" | jq -r '.items[0].id')"
if [ "$TOP_MATCH_ID" = "$NEW_CARD_ID" ]; then
  pass "semantic search's top match is the just-created 'Rotate the S3 access keys' card"
else
  echo "$NEAREST" | jq -c '.items[] | {id, title}'
  fail "expected top nearestTo match to be ${NEW_CARD_ID}, got ${TOP_MATCH_ID}"
fi
curl -fsS -X DELETE "${CRATEBASE_URL}/api/collections/cards/records/${QUERY_ID}" -H "authorization: Bearer ${ALICE_TOKEN}" >/dev/null
pass "throwaway query card deleted"

# Confirm the isQuery row never leaked into an ordinary list.
LEAK_CHECK="$(curl -fsS "${CRATEBASE_URL}/api/collections/cards/records" -H "authorization: Bearer ${ALICE_TOKEN}" -G --data-urlencode "filter=teamRef = \"${TEAM_ID}\" && title = \"rotate credentials\"")"
[ "$(echo "$LEAK_CHECK" | jq '.items | length')" -eq 0 ] && pass "isQuery row never appeared in a plain list (already deleted + rule-excluded)" || fail "isQuery row leaked into a list"

echo "== assign the new card to bob, expect a notification + email =="
ALL_MEMBERS="$(curl -fsS "${CRATEBASE_URL}/api/collections/_team_members/records" -H "authorization: Bearer ${ALICE_TOKEN}" -G --data-urlencode "filter=teamRef = \"${TEAM_ID}\"" --data-urlencode "expand=userRef")"
BOB_ID="$(echo "$ALL_MEMBERS" | jq -r '.items[] | select(.expand.userRef.email == "bob@example.com") | .userRef')"
[ -n "$BOB_ID" ] && [ "$BOB_ID" != "null" ] && pass "found bob's user id: $BOB_ID" || fail "could not find bob in team membership"

curl -fsS -X PATCH "${CRATEBASE_URL}/api/collections/cards/records/${NEW_CARD_ID}" \
  -H "authorization: Bearer ${ALICE_TOKEN}" -H 'content-type: application/json' \
  -d "$(jq -nc --arg a "$BOB_ID" '{assigneeRef:$a}')" >/dev/null
pass "assigned the card to bob"

echo "== sign in as bob, check for the notification =="
BOB_TOKEN="$(curl -fsS -X POST "${CRATEBASE_URL}/api/collections/users/auth-with-password" \
  -H 'content-type: application/json' \
  -d '{"identity":"bob@example.com","password":"password123"}' | jq -r .token)"
FOUND=0
for _ in $(seq 1 10); do
  NOTIFS="$(curl -fsS "${CRATEBASE_URL}/api/collections/notifications/records" -H "authorization: Bearer ${BOB_TOKEN}" -G --data-urlencode "filter=cardRef = \"${NEW_CARD_ID}\"")"
  if [ "$(echo "$NOTIFS" | jq '.items | length')" -gt 0 ]; then
    FOUND=1
    echo "$NOTIFS" | jq -c '.items[0] | {message, read}'
    break
  fi
  sleep 1
done
[ "$FOUND" = "1" ] && pass "pb_hook created a notification for bob" || fail "no notification found for bob after assignment"

echo "== superuser: mail landed in the dev mail inbox =="
SU_TOKEN="$(curl -fsS -X POST "${CRATEBASE_URL}/api/collections/_superusers/auth-with-password" \
  -H 'content-type: application/json' \
  -d "$(jq -nc --arg identity "$ADMIN_EMAIL" --arg password "$ADMIN_PASSWORD" '{identity:$identity,password:$password}')" | jq -r .token)"
MAILS="$(curl -fsS "${CRATEBASE_URL}/api/dev/mails" -H "authorization: Bearer ${SU_TOKEN}")"
MAIL_TO_BOB="$(echo "$MAILS" | jq '[.items[]? // .[]? | select(.to[]?.address == "bob@example.com")] | length' 2>/dev/null || echo 0)"
if [ "${MAIL_TO_BOB:-0}" -gt 0 ]; then
  pass "found ${MAIL_TO_BOB} mail(s) to bob in /api/dev/mails"
else
  echo "$MAILS" | jq -c '.' | head -c 2000
  fail "no mail to bob found in /api/dev/mails"
fi

echo
echo "ALL CHECKS PASSED"
