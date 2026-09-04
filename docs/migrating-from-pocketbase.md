# Migrating from PocketBase

`cratebase migrate-from-pocketbase` is a one-shot import of an existing
PocketBase installation's **collections, records and files** into a
Cratebase instance. It reads PocketBase's `pb_data` directly (its SQLite
file and its `storage/` folder) — no running PocketBase server is required,
and nothing on the PocketBase side is modified.

This document describes exactly what the command does, what it verifiably
does and does not carry across, and walks through a real run against a real
PocketBase v0.40.2 instance.

## The command

```bash
cratebase migrate-from-pocketbase <path-to-pb_data> --dir <cratebase-data-dir>
```

* `<path-to-pb_data>` is PocketBase's data directory — the same one
  `pocketbase serve --dir <path>` points at. It must contain `data.db` and,
  if any collection has file fields, a `storage/` subdirectory.
* `--dir <cratebase-data-dir>` is the Cratebase data directory to migrate
  *into*. It is bootstrapped (created fresh, or added to if it already
  exists) exactly like `cratebase serve --dir` would.

The command prints a report: which collections were created vs. had fields
merged into an existing schema, which PocketBase system collections were
skipped and why, any field with no Cratebase equivalent (flagged, never
silently dropped), any OAuth2 provider that needs re-configuring, a
per-collection record count, and the number of files copied.

## Why almost nothing needs "translating"

Cratebase's collection/field JSON is deliberately the exact shape
PocketBase v0.23+ uses (same field option names, same auth-config layout).
More importantly, **Cratebase reimplements PocketBase's own id-derivation
algorithm**: a collection's id is `pbc_<crc32(type + name)>` and a field's
id is `<type><crc32(name)>`, the same deterministic formula PocketBase uses
internally (`crates/core/src/ids.rs`). This was verified directly, not
assumed: a `categories` (base) collection created independently in both a
real PocketBase v0.40.2 instance and a fresh Cratebase instance gets the
identical id `pbc_3292755704` in both.

The practical consequence: a `relation` field's `collectionId`, copied
byte-for-byte out of PocketBase's dump, already points at the right place
in the new Cratebase database — **no id-remapping table is needed**, even
though the migration tool never coordinates collection creation order.
PocketBase's default `users` collection (`_pb_users_auth_`) and the
`_superusers`/`_mfas`/`_otps`/`_externalAuths`/`_authOrigins` system
collections get the same fixed ids in both systems for the same reason.

What genuinely needs translating:

* PocketBase's `system` fields (`id`, `password`, `tokenKey`, `email`,
  `emailVisibility`, `verified`) are stripped from the migrated payload —
  Cratebase's own collection-creation path regenerates them identically for
  the collection's type, so resubmitting them is both unnecessary and (for
  `id`) rejected as an attempt to redefine a reserved field.
* Auth-collection config PocketBase stores in `_collections.options`
  (`authRule`, `passwordAuth`, `mfa`, `otp`, `authAlert`) is flattened onto
  the collection JSON's top level, which is where Cratebase expects it.
* Each column's raw SQLite value is decoded against the field's declared
  type. SQLite enforces no real column typing, so a `select`/`file`/
  `relation` field is a bare string when `maxSelect<=1` and a JSON array
  otherwise — confirmed against a real `pocketbase serve` database dump
  (`sqlite3 data.db '.schema'`), not assumed.
* Any field type this tool does not recognise is **flagged in the report as
  unsupported and skipped**, never silently dropped. In practice this
  should never fire against a real PocketBase v0.23+ export: every
  PocketBase field type (`text`, `editor`, `number`, `bool`, `email`,
  `url`, `date`, `autodate`, `select`, `file`, `relation`, `json`,
  `password`, `geoPoint`) has a Cratebase field of the same name and
  shape. Cratebase's `vector` field type has no PocketBase equivalent, but
  that is an addition on the way in, not something migration could ever
  lose.
* The reverse case also happens: Cratebase's `_superusers` collection has
  a `role` field (`owner`/`admin`) PocketBase has no concept of, since
  PocketBase has exactly one superuser tier. A migrated superuser is
  assigned `role: "owner"` — full access, the same default Cratebase's own
  upgrade migration backfills onto pre-existing superuser rows, so
  migrating in never leaves an operator with reduced access.

## What is intentionally NOT migrated

* **PocketBase's session/auth tokens.** They are signed with a secret this
  tool never reads out of `pb_data` (and, if it did, must never reuse
  across installations). Every existing user and superuser must log in
  again after migrating — but see below, their **password** keeps working
  without a reset.
* **OAuth2 provider client secrets.** PocketBase's `_collections.options`
  does not round-trip OAuth2 client secrets over its own dashboard-driven
  JSON in the first place, and reusing one across two separate app
  registrations would not work regardless. If the source collection had
  any OAuth2 provider configured, the collection is created with
  `oauth2.enabled: false` and flagged in the migration report; reconfigure
  providers with fresh credentials after migrating.
* **`_mfas`, `_otps`, `_externalAuths`, `_authOrigins`.** Ephemeral
  second-factor/session state, meaningless without the exact signing
  secrets behind it (see above). Users re-establish MFA and OAuth2 links
  after migrating; standing password/email auth is unaffected.
* **PocketBase JS migrations and hooks** (`pb_migrations/`, `pb_hooks/`).
  Out of scope — this tool moves data, not server-side application code.
  Port hook logic by hand if you have any.

## Password compatibility: verified, not assumed

This matters enough to state plainly, backed by what was actually checked:

**Cratebase's password verification accepts PocketBase's bcrypt hashes
directly.** `crates/auth/src/password.rs` dispatches on the hash prefix:
`$argon2...` (Cratebase's own new hashes) or `$2a$`/`$2b$`/`$2y$` (bcrypt,
exactly what a real PocketBase v0.40.2 `_superusers`/`users` table stores —
confirmed by dumping `sqlite3 pb_data/data.db "SELECT password FROM
users"` and seeing `$2a$10$...` rows). A migrated user's PocketBase
password is copied across unchanged and **logs in immediately** with no
forced reset. The one-time cost is transparent to the user: the next
successful login re-hashes the password to Argon2id in the background
(`needs_rehash`/`hash_password` in the same file), so PocketBase's bcrypt
hash is only ever used once, right after migration.

This was verified end-to-end (see the worked example below), not inferred
from reading the code alone: a real PocketBase superuser and a real
PocketBase auth-collection user were migrated, and both logged into the
migrated Cratebase instance **using their original PocketBase passwords**,
with a `200` and a valid auth token both times.

The one real limitation: since session tokens are not migrated (see
above), a user who was logged in on the PocketBase side needs to log in
again — but with the password they already know, not a reset.

## Worked example

This is a real run, not a hypothetical — every command below was actually
executed against a real `pocketbase` v0.40.2 binary and a real
`cratebase` build.

### 1. Set up a real PocketBase instance with realistic data

```bash
mkdir -p pb_data
pocketbase serve --http=127.0.0.1:8090 --dir=pb_data &
pocketbase superuser create admin@example.com adminpass123 --dir=pb_data
```

Through the API (a dashboard session does the same thing), two collections
were created with a realistic field mix:

* `categories` (base): `title` (text), `kind` (select, single).
* `books` (base): `title` (text), `pages` (number), `in_print` (bool),
  `category` (relation → `categories`), `cover` (file), `tags` (select,
  multi), `rating` (number).

Then real data: two categories, three books (each with an uploaded cover
image and a relation to a category), and one `users` auth record (`alice`)
with an uploaded avatar — via `POST` with `multipart/form-data`, exactly
like a real client SDK would upload.

### 2. Inspect PocketBase's real internal layout

```
$ sqlite3 pb_data/data.db "SELECT id,name,type,system FROM _collections"
pbc_2279338944|_mfas|base|1
pbc_1638494021|_otps|base|1
pbc_2281828961|_externalAuths|base|1
pbc_4275539003|_authOrigins|base|1
pbc_3142635823|_superusers|auth|1
_pb_users_auth_|users|auth|0
pbc_3292755704|categories|base|0
pbc_2170393721|books|base|0

$ sqlite3 pb_data/data.db "SELECT * FROM books"
urz7wys0nsizdf5|dune_cover_aaqw9pr748.png|k7zi7ewdor8vhwh|1|412|5|["classic","award-winning"]|Dune
...

$ find pb_data/storage -type f
pb_data/storage/pbc_2170393721/k7zi7ewdor8vhwh/dune_cover_aaqw9pr748.png
pb_data/storage/pbc_2170393721/k7zi7ewdor8vhwh/dune_cover_aaqw9pr748.png.attrs
...
```

(the `.attrs` sidecar files are `object_store`/blob-store metadata, not
migrated — Cratebase's own object store does not need them.)

### 3. Run the migration

```bash
$ cratebase migrate-from-pocketbase pb_data --dir cratebase_data
collections created: 2
  + categories
  + books
collections updated (existing schema, fields merged): 1
  ~ users
PocketBase system collections skipped (already exist identically in Cratebase, or are ephemeral auth state not carried across): _mfas, _otps, _externalAuths, _authOrigins
records migrated:
  _superusers: 1
  books: 3
  categories: 2
  users: 1
files copied: 4
```

`users` shows up as "updated" rather than "created" because Cratebase
already seeds a `users` collection with the same id and the same default
`name`/`avatar` fields on bootstrap — the migration only had to confirm
nothing new needed adding for this dataset (a real installation with
custom fields on `users` would see those fields appended).

### 4. Verify from Cratebase, over real HTTP

```bash
$ cratebase serve --http=127.0.0.1:8095 --dir=cratebase_data &
```

Superuser login with the **original PocketBase password**:

```
POST /api/collections/_superusers/auth-with-password
{"identity":"admin@example.com","password":"adminpass123"}
→ 200, valid JWT
```

Migrated user login, also with the **original PocketBase password**:

```
POST /api/collections/users/auth-with-password
{"identity":"alice@example.com","password":"alicepass123"}
→ 200, record id jt70gjgj95oiaz0 (same id PocketBase assigned)
```

Records, with relation expansion, exactly matching what PocketBase had:

```
GET /api/collections/books/records?expand=category&sort=title
→ Dune (id k7zi7ewdor8vhwh, category expands to "Science Fiction"),
  Foundation, A Brief History of Time — same ids, same tags arrays,
  same in_print booleans, same cover filenames as the original
  PocketBase database.
```

File download round-trip — not just "the metadata says a file exists", the
actual bytes:

```
GET /api/files/books/k7zi7ewdor8vhwh/dune_cover_aaqw9pr748.png
GET /api/files/users/jt70gjgj95oiaz0/alice_avatar_suv9bdbxfz.png
→ both 200, both byte-for-byte identical to the file originally
  uploaded to PocketBase.
```

`created`/`updated` timestamps on migrated records also match PocketBase's
originals exactly (the migration tool patches them back after insert,
since a plain record write would otherwise stamp "now").

### Collection and record ids did not change

Because of the deterministic id derivation described above, `books` kept
PocketBase's id `pbc_2170393721`, `categories` kept `pbc_3292755704`, and
every record kept its original PocketBase id. A client that already stores
record ids (bookmarks, deep links, cached data) keeps working unmodified
against the migrated Cratebase instance, using collection **names** in
URLs (`/api/files/books/...`) as `pocketbase`-family SDKs already do —
Cratebase resolves both collection ids and names.

## Known limitations, restated

* Existing PocketBase session tokens are invalid after migration; users
  must log in again (their password works unchanged).
* OAuth2 providers are disabled on migrated auth collections and must be
  reconfigured with fresh client credentials.
* `_mfas`/`_otps`/`_externalAuths`/`_authOrigins` rows are not migrated —
  re-establish MFA/external logins post-migration.
* PocketBase JS hooks/migrations are not touched; port custom
  server-side logic by hand.
* View collections' `viewQuery` is carried across on a best-effort basis
  (translated from `_collections.options.query`) but was not exercised in
  the worked example above, since the test PocketBase instance had no view
  collections — verify a migrated view collection's query still resolves
  correctly for your own schema before relying on it.
