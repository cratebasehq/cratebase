# @cratebase/schema

Schema-as-code for Cratebase: typed field builders, deterministic collection ids and a
`defineSchema()` that emits the `{ "collections": [...] }` document accepted by
`cratebase dev` (`./schema.json`), `cratebase schema` and `POST /api/schema/apply`.
Record types still come from `cratebase typegen`.

```ts
// schema.ts
import { defineCollection, defineSchema, idx, relation, select, text, writeSchema } from "@cratebase/schema";

const posts = defineCollection({
  name: "posts",
  fields: [
    text("title", { required: true, searchable: true }),
    select("status", ["draft", "live"]),
    relation("author", "users"), // target by name; resolved to its collection id
  ],
  indexes: [idx("idx_posts_title", "posts", ["title"], { unique: true })],
  listRule: "", // "" = public; omitted rules are superuser-only (null)
});

await writeSchema(defineSchema(posts), new URL("./schema.json", import.meta.url));
```

Run `bun schema.ts`, then `cratebase dev` applies `schema.json`.

- Builders: `text, editor, number, bool, email, url, date, autodate, select, file, relation, json, password, geoPoint, vector`.
- `collectionId(type, name)` is `pbc_<crc32(type + name)>`, identical to the server's id.
- Relation targets may be any collection in the same `defineSchema(...)` call, or the built-ins `users` / `_superusers`.
- `id`, `created` and `updated` fields are added by the server; do not declare them.
