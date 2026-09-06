# @cratebase/schema-codegen

Build-time CLI that turns a [Cratebase](https://github.com/cratebasehq/cratebase) schema-as-code
JSON document into a single `.d.ts` file with one TypeScript `interface` per non-system
collection. It has no dependency on the server or admin dashboard — run it from a project's own
build pipeline, wherever a `PostsRecord`-shaped type is more useful than hand-written ones.

```
npx @cratebase/schema-codegen ./schema.json -o src/cratebase-types.d.ts
```

The input is exactly what `GET /api/collections` returns (an `{ items: [...] }` page), what
`PUT /api/collections/import` and `POST /api/schema/apply` accept (`{ collections: [...] }`), a
bare array of collection objects, or a live server URL to fetch that page from directly:

```
npx @cratebase/schema-codegen https://api.example.com --token <superuser-jwt> -o types.d.ts
```

Field types follow `FieldKind`'s variants in `crates/core/src/field.rs` one-for-one, so a change
to that enum is the only place the mapping needs to catch up. `select`, `file` and `relation`
fields become an array (`string[]`) when their `maxSelect` is greater than 1, and a bare `string`
otherwise; every other field kind maps straight to its TypeScript scalar (`text` → `string`,
`number` → `number`, `bool` → `boolean`, `json` → `unknown`, `geoPoint` →
`{ lon: number; lat: number }`, ...).

## Options

```
cratebase-codegen <schema.json | server-url> [-o out.d.ts] [--token <jwt>]
```

- `<schema.json | server-url>` — a local file path, or an `http(s)://` server origin (or its
  `/api/collections` URL directly).
- `-o, --out` — output path (default `cratebase-types.d.ts`); may also be given positionally.
- `--token` — a superuser JWT, sent as `Authorization: Bearer <token>` when fetching from a live
  server.

## Development

```
bun install
bun run typecheck
bun run build   # emits dist/codegen.js, the package's published bin entry point
```
