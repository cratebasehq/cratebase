# Known divergences, skips and PocketBase surprises

Status against **PocketBase v0.40.2** with the **`pocketbase` JS SDK 0.28.0**:
**180 passing, 1 skipped, 0 failing** against a PocketBase server; against a
Cratebase server, 181 of the same tests pass (the two collection/settings
list assertions widen for Cratebase's extra system collections/settings key,
see below) plus the value-add feature suites tracked separately.

## Skipped tests

| Test | Reason |
| --- | --- |
| `backups.test.ts` → *restore replaces the data directory* | `POST /api/backups/{key}/restore` swaps `pb_data` and restarts the app, which would kill the server mid-run and invalidate every other test. The error paths (`restore` of a missing backup, upload of a non-zip) are asserted instead. |

Additionally, six auth tests are **conditionally** skipped when the SMTP sink is
off (`MAIL_SINK=0`, or an external `BASE_URL` without `MAIL_SINK=1`), because
they need to read an emailed token/code: verification, password reset and email
change happy paths, the auth-alert email, the OTP happy path and MFA completion
via OTP. With the harness' built-in sink (the default when the harness spawns
the server) they run and pass against PocketBase. There is no PocketBase HTTP
API that exposes a sent OTP code, so a mail sink is the only way to test those.

## PocketBase behaviours the Rust implementation must match

These are all asserted by the suite. Several contradict the docs or intuition.

### Collections

1. **New auth collections default to different durations than the built-in
   `users` collection**: `authToken.duration` 432000 (5 days),
   `verificationToken` 86400, `passwordResetToken` / `emailChangeToken` 1800,
   `fileToken` 180, `mfa.duration` **600** (not 1800), `otp.duration` 180,
   `otp.length` 8.
2. **Token secrets are never serialized** — `authToken` and friends come back as
   `{duration}` only.
3. **View collection fields get synthetic ids** of the form `_clone_XXXX`, not
   the origin collection's field ids. Writes to a view return
   `400 "Unsupported collection type."`; a view query without an `id` column is
   `validation_invalid_view_query`.
4. **The base scaffold has only the `id` field** (`GET /api/collections/meta/scaffolds`);
   `created`/`updated` autodate fields are *not* part of it. The auth scaffold
   has `id, password, tokenKey, email, emailVisibility, verified` and two
   indexes whose table name is empty (`ON `` (`tokenKey`)`).
5. **Duplicate field names are collapsed silently** — the last definition wins,
   no validation error.
6. **An unknown field `type` is a body-parse error**, not a field validation
   error: `400 "Failed to load the submitted data due to invalid formatting."`
   with `data: {}`.
7. **Errors on list-typed properties are keyed by index**:
   `data.indexes["0"].code = "validation_invalid_index_expression"`. An index on
   a missing column reports the raw SQLite error in `message`
   (`"Failed to create index idx_x - SQL logic error: no such column: missing (1)."`).
8. **Deleting a referenced collection** returns a message naming the referrer:
   `"Failed to delete collection probably due to existing reference in <name>."`
9. `type` cannot change after creation: `validation_collection_type_change`.

### Records / filters

10. **`= `is case-sensitive, `~` (LIKE) is not.** `title = "first post"` does not
    match `"First post"`, but `title ~ "first"` does.
11. **The `?` "any of" prefix only affects joined identifiers** (relations,
    back-relations, `@collection.X`). On a plain multi-value column such as a
    multi-select, `tags ?= "rust"` compares against the JSON *text*
    `["go","rust"]` and never matches. `tags:each ?= "rust"` is the working
    form; `tags:each = "rust"` means *every* element equals.
12. **Back-relations follow the all/any rule too**: `comments_via_post.text = "nice"`
    requires *every* joined comment to equal `"nice"`; use `?=` for "at least
    one". A record with no joined rows compares as `""`, so `x_via_y.id = ""`
    finds them — `:length = 0` does **not**.
13. **Type coercion instead of validation** in several places:
    `views: "abc"` on a number field stores `0`; an unparsable date stores `""`;
    a single-select given `["news","blog"]` keeps the **last** element. Only
    decimals on `onlyInt` (`validation_only_int_constraint`, *"Decimal numbers
    are not allowed."*) and min/max are actually rejected.
14. **A failing `listRule` is not an error** — it yields an empty list. A failing
    `viewRule`/`updateRule`/`deleteRule` is a **404**, a failing `createRule` a
    **400 "Failed to create record."** with `data: {}`, and a `null` rule
    (superuser only) a **403 "Only superusers can perform this action."**
15. **A client-supplied `id` is validated as a text field**:
    `validation_min_text_constraint` when too short and
    `validation_invalid_format` when it violates `^[a-z0-9]+$`.
16. **Validation code names that differ from the obvious guess**: URLs are
    `validation_invalid_url` / *"Must be a valid url."*; date bounds are
    `validation_min_greater_equal_than_required` /
    `validation_max_less_equal_than_required` (with a `params.threshold` and a
    Go-formatted time in the message); too many select values is
    `validation_too_many_values` / *"Select no more than 2."*; too many files is
    `validation_too_many_files`.
17. **Malformed JSON on a records endpoint** returns the generic
    *"Something went wrong while processing your request."*, while the same body
    on a collections endpoint returns *"Failed to load the submitted data due to
    invalid formatting."*
18. `perPage` is capped at **1000** and `page: 0` becomes `1`.

### Auth

19. **A disabled auth method is a 403, not a 400**:
    *"The collection is not configured to allow password authentication."* /
    *"... OTP authentication."* Auth endpoints on a **base** collection are
    **404 "Missing or invalid auth collection context."**, but
    `listAuthMethods` on a *non-existent* collection is
    **404 "Missing or invalid collection context."** (different wording).
20. **Hidden emails are omitted, not blanked** — a record whose
    `emailVisibility` is false comes back with **no `email` key at all** for
    unauthorized viewers (self, superusers and `manageRule` holders see it).
21. **Setting `verified` or `email` on yourself without manage access** fails
    with `validation_values_mismatch` / *"Values don't match."* (not a
    `not_allowed` code).
22. **MFA second call**: repeating the *same* method returns
    `400 "A different authentication method is required."` — the `mfaId` is
    **not** echoed. An unknown `mfaId` is `400 "Invalid or expired MFA session."`
    with `data: {}` (not a field error). OTP used alone under MFA is itself a
    first factor and returns `401 {mfaId}` with `_mfas.method = "otp"`.
23. **Enabling MFA without a second auth method** is rejected with
    `data.mfa.enabled.code = "validation_mfa_not_enough_auths"`.
24. **`authWithOTP` failures are `400 "Invalid or expired OTP."` with `data: {}`**,
    not the generic *"Failed to authenticate."* `requestOTP` for an unknown
    email still returns a well-formed fake `otpId`.
25. **Impersonation tokens are not refreshable, but `auth-refresh` does not
    fail** — it returns the *same* token back.
26. **A superuser token cannot refresh another collection's session**:
    `403 "The request requires auth record from _superusers collection."`
27. `confirmVerification` distinguishes *garbage* tokens
    (`validation_invalid_token_claims` / *"Missing email token claim."*) from
    *well-formed but unsigned* ones (`validation_invalid_token`). The wrapper
    message for all confirm endpoints is
    *"An error occurred while validating the submitted data."*
28. `requestEmailChange` to your own address is
    `validation_not_in_invalid` / *"Must not be in list."*; confirming an email
    change rotates `tokenKey`, invalidating existing sessions (as does any
    password change, by the owner or by a superuser).

### Files

29. **`protected: true` does not mean "a token is always required."** PocketBase
    evaluates the collection's `viewRule` for the token owner, and an empty rule
    is public — a protected file in a public collection is served to anyone.
    Only a restricted `viewRule` gates it, and then an unauthorized request is a
    **404** (not 403).
30. **Stored file names pad short basenames**: `a.txt` becomes
    `a<random>_<10 rand>.txt`, `notes.txt` becomes `notes_<10 rand>.txt`.
31. **`Content-Disposition` is always present** — `inline; filename="…"` without
    `?download=1`, `attachment; filename="…"` with it.
32. **Thumb sizes outside the field's `thumbs` list are generated on demand**,
    and requesting a thumb of a non-image falls back to the original file (200).
33. File size errors are per-file:
    *"Failed to upload big.txt - the maximum allowed file size is 5 bytes."*

### Batch / settings / logs / crons / backups

34. **Batch errors nest one level deeper than the docs suggest**:
    `{status: 400, message: "Batch transaction failed.", data: {requests: {"<index>": {code: "batch_request_failed", message: "Batch request failed.", response: {status, message, data}}}}}`.
    Only the failing index appears. `maxRequests` overflow is
    `validation_length_too_long`; an empty batch is `validation_required`;
    batching while disabled is `403 "Batch requests are not allowed."`
35. **Successful deletes inside a batch report `{status: 204, body: null}`**;
    everything else `{status: 200, body: <record>}`.
36. **Settings validation** wraps errors in
    *"An error occurred while saving the new settings."*; `testS3` on a disabled
    filesystem returns a multi-line message
    (*"Failed to test the S3 filesystem. Raw error: … not enabled."*).
37. **Log levels are slog levels**: a 200 request is `level 0`, a failed request
    is `level 8` and carries `data.error` with the error message. `data.auth` is
    `""` for guests (not `"guest"`). The SDK types `LogModel.level` as `string`
    while the API returns a **number** — an SDK typing bug, harmless at runtime.
38. **Backup download without a token is `403 "Insufficient permissions to
    access the resource."`**; deleting a missing backup returns a multi-line
    *"Invalid or already deleted backup file. Raw error: …"*; restoring a
    missing one is *"Missing or invalid backup file."*
39. **The scheduled-backup cron id is `__pbAutoBackup__`** (appears only once
    `settings.backups.cron` is set); the always-present jobs are
    `__pbDBOptimize__`, `__pbMFACleanup__`, `__pbOTPCleanup__`,
    `__pbLogsCleanup__`. Running an unknown job is
    `404 "Missing or invalid cron job."`
40. **A wrong HTTP method on an existing route is a 404, not a 405.**
41. **The realtime client id is a 40-char alphanumeric string**, not a 15-char
    record id.
42. `/api/health` is the only endpoint that still uses `code` instead of
    `status` in its envelope.

## Cratebase-only additions (not present in PocketBase)

These are new capabilities, not quirks to match, so they widen the
expected-value lists in `collections.test.ts`/`settings.test.ts` rather than
describing a behavioural difference on a shared surface.

43. **Five extra system collections**, all superuser-only end to end (same
    trust tier as `_superusers`/`_mfas`): `_cron_jobs` (custom scheduled SQL
    jobs), `_llm_usage` (persisted LLM gateway chat history), `_team_members`
    and `_teams` (workspace membership), `_webhooks` (outgoing webhook
    config).
44. **An extra top-level settings key, `llm`**: the LLM chat gateway's
    provider config (`baseUrl`, `apiKey`, `model`). `apiKey` is stripped from
    `GET /api/settings` the same way `smtp.password`/`s3.secret` are.

## Suite-side workarounds (not PocketBase behaviour)

* Bun's test runtime has **no global `EventSource`**, so `harness.ts` ships a
  minimal SSE implementation rather than depending on the `eventsource` package.
* PocketBase's **automigrate** writes JS migrations to `<dataDir>/../pb_migrations`
  and replays them into later runs; the harness disables it and redirects the
  migrations/hooks/public dirs into the temp data dir.
