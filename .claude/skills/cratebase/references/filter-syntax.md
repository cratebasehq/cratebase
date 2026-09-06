# Filter expression language

Used in two places, with identical syntax and semantics: API rules
(`listRule`, `viewRule`, `createRule`, `updateRule`, `deleteRule` on a
collection) and the `?filter=` query param on list requests. Source of
truth: `crates/filter/src/` (lexer → parser → AST in `ast.rs` → compiler
in `compiler.rs`, which emits parameterized SQL). Because rules and
`?filter=` share a grammar, you can prototype a rule by hitting
`GET /api/collections/{c}/records?filter=<expr>` directly before pasting
it into a `*Rule` field.

## Grammar

```text
expr     := or
or       := and ( "||" and )*
and      := unary ( "&&" unary )*
unary    := "(" expr ")" | compare
compare  := operand OP operand
OP       := = != > >= < <= ~ !~ ?= ?!= ?> ?>= ?< ?<= ?~ ?!~
operand  := literal | ident modifier? | call
literal  := string | number | true | false | null
ident    := [@]?[A-Za-z_][A-Za-z0-9_]* ( "." [A-Za-z0-9_]+ )*
modifier := ":isset" | ":length" | ":each" | ":lower"
call     := name "(" operand ( "," operand )* ")"
```

(`crates/filter/src/ast.rs:1-15`)

## Operators

- `=`, `!=`, `>`, `>=`, `<`, `<=` — standard comparison.
- `~`, `!~` — substring/LIKE match, case-insensitive (`~`) and its
  negation.
- **`?`-prefixed "any of" variants** (`?=`, `?!=`, `?>`, `?>=`, `?<`,
  `?<=`, `?~`, `?!~`) vs. the **bare operator on an element set** (a path
  through a multi relation, or a scalar unpacked with `:each`): `?op`
  means "true if **any** element matches"; the bare op on that same
  element set means "true if **every** element matches" (plus at least
  one element, unless the operator itself is satisfied by the empty
  case, e.g. `!=` or `= ""`). E.g. `tags.name ?= "public"` matches if
  *any* linked tag is `"public"`; `tags.name = "public"` through that
  same relation requires *every* linked tag to be `"public"`.
  A **bare multi-valued column with no relation path or `:each` is not
  an element set at all** — it compares as the raw JSON text of the
  column (`tags = "a"` is false for `["a","b"]`), matching PocketBase.
  (`crates/filter/src/compiler.rs:17-24`)

## Literals

Strings (with escapes), numbers, `true`/`false`, `null` — standard.

## Modifiers (`:suffix` on an identifier)

- `:isset` — true if the field/path was present in the request payload
  (useful in `createRule`/`updateRule` to require or forbid a client
  setting a field).
- `:length` — length of a string, or the element count of a multi-value
  field.
- `:each` — opt-in to unpack a multi-valued *scalar* field (one not
  already reached through a multi relation) into an element set, so
  bare/`?`-prefixed operators apply per-element instead of to the raw
  JSON text of the column (`crates/filter/src/compiler.rs:262-266,296-302`).
- `:lower` — lowercase the value before comparing (e.g.
  `@request.auth.email:lower = 'a@b.c'`).

Modifiers cannot be applied to `@request.auth.id` directly in a way that
changes the type, and `@now:lower` is a compile error — modifiers only
apply where they make semantic sense (`crates/filter/src/tests.rs:494-501`).

## Date macros

All computed in UTC at compile time and bound as SQL parameters, in
PocketBase's `YYYY-MM-DD HH:MM:SS.sssZ` format (or as integers for the
calendar-component macros):

```
@now @second @minute @hour @weekday @day @month @year
@yesterday @tomorrow @todayStart @todayEnd
@monthStart @monthEnd @yearStart @yearEnd
```

(`crates/filter/src/lib.rs:21-23`, `crates/filter/src/macros.rs:1-3`)

Example: `published_at < @now` compiles to
`"posts"."published_at" < $1` with `$1` bound to the current UTC
timestamp (`crates/filter/src/tests.rs:838-841,881-884`).

## `@request.*` context variables

- `@request.auth` / `@request.auth.<path>` — the authenticated record (or
  empty/falsy fields if the request is anonymous — **not** an error).
  `@request.auth.collectionName`, `@request.auth.verified`, and any
  custom field on the auth collection are reachable this way.
- `@request.body.<path>` (alias `@request.data.<path>`, deprecated but
  still supported) — the request payload being created/updated. Use this
  in `createRule`/`updateRule` to constrain what a client is allowed to
  write, e.g. `@request.body.status = "draft"` to force new records to
  start as drafts regardless of what else the client sends.
- `@request.query.<path>` — query string params on the current request.
- `@request.headers.<name>` — request header, name lowercased with `-`
  replaced by `_` (PocketBase convention), e.g.
  `@request.headers.x_token = 'tok'` for an `X-Token` header.
- `@request.method` — HTTP method as an uppercase string (`'GET'`,
  `'POST'`, ...).
- `@request.context` — the API context the rule is being evaluated in
  (e.g. `"realtime"` for realtime subscription auth checks).

(`crates/filter/src/lib.rs:21-26`, `crates/filter/src/resolver.rs:19-28`)

### Anonymous-request behavior (read this before shipping an owner-style rule)

`@request.auth.*` doesn't error out for unauthenticated requests, it
degrades to empty/false, and Cratebase compiles the *comparison*
accordingly rather than leaving a dangling parameter:

```text
"@request.auth.id != ''"                          → "1 = 0"      (anon)
"@request.auth = true"                            → "1 = 0"      (anon)
"author = @request.auth.id"                       → ("posts"."author" = '' OR "posts"."author" IS NULL)  (anon)
"@request.auth.id != '' && author = @request.auth.id"
                                                   → (1 = 0 AND (...))  (anon)
```

(`crates/filter/src/tests.rs:966-977`)

So a rule like `author = @request.auth.id` is already safe for anonymous
requests without an explicit `@request.auth.id != ''` guard — it just
matches nothing. Add the explicit guard only when you need short-circuit
clarity or are combining it with an `||` where the implicit `false`
wouldn't be enough.

## Relations and `@collection.*`

- **Dot-notation through a relation field** reaches fields on the related
  record: `author.name`, `comments_via_post.title` (the latter is a
  *back-relation* — a relation field on another collection pointing at
  this one, addressed by `<field>_via_<relationField>`).
- **`@collection.<name>.<path>`** reaches an arbitrary other collection by
  name (not just ones directly related to the current record) — useful
  for membership/permission checks that don't have a direct relation
  field. Real example, checking a join-table-style membership collection
  from a `teams`-scoped rule:

  ```text
  @collection.memberships.user ?= @request.auth.id
    && @collection.memberships.team ?= title
  ```

  (`crates/filter/src/tests.rs:676-679`)

## Function calls

Currently one built-in: `geoDistance(lonA, latA, lonB, latB)`, returning
distance for comparison against a threshold:

```text
geoDistance(loc.lon, loc.lat, @request.query.lon, @request.query.lat) <= 5
```

(`crates/filter/src/ast.rs:93-95`, `crates/filter/src/tests.rs:1037-1040`)

## Worked examples

```text
status = "active" && (author = @request.auth.id || tags.name ?= "public")
```
(`crates/filter/src/lib.rs:5-6`) — public if tagged `"public"`, otherwise
owner-only, and only when `status = "active"`.

```text
owner = @request.auth.id
```
The default single-owner pattern — combine with `null` on
`deleteRule`/`updateRule` if only the owner should ever mutate, or leave
as-is if any authenticated user of a different collection also needs
access via a relation check.

```text
@request.method = 'GET'
```
Rules can also gate on HTTP method, though this is unusual — most rules
should express *data* ownership, not transport details.
