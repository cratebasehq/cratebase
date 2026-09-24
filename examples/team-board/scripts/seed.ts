#!/usr/bin/env bun
// Seeds demo data (a "demo team" story: Acme Inc with alice/bob, plus a
// second team Globex Corp with carol for exercising cross-team access
// denial) from seed.json, using `@cratebase/client` as superuser — same
// SDK the app itself uses, no separate admin tooling.
//
// seed.json is `{ "<collectionName>": [record, ...] }`, deliberately the
// same shape a future `cratebase seed` command (mentioned as
// in-progress, not depended on here) is expected to accept, so switching
// to it later should just mean deleting this file's resolution logic and
// keeping seed.json. Two conventions on top of plain record data:
//   - `"$id": "alias"` on a record marks it referenceable by later records.
//   - any string field value of the exact form `"$alias"` is resolved to
//     that earlier record's real id before the request is sent (record
//     ids are per-run random, unlike collection ids — see
//     scripts/lib/ids.ts's doc comment — so seed.json can't hardcode them).
// Collections are seeded in the JSON's own key order, so list dependent
// collections (e.g. "cards") after what they reference (e.g. "columns").
// `@cratebase/client` isn't published to npm yet (see README.md) — this
// script runs directly under `bun` (not through Vite, so the app's own
// `@cratebase/client` alias in vite.config.ts doesn't apply here) and
// resolves the SDK straight from its monorepo source instead.
import { createClient, CratebaseError } from "../../../sdk/js/client/src/index.js";

const BASE_URL = process.env.CRATEBASE_URL ?? "http://localhost:8090";
const SUPERUSER_EMAIL = process.env.SUPERUSER_EMAIL ?? "admin@example.com";
const SUPERUSER_PASSWORD = process.env.SUPERUSER_PASSWORD ?? "changeme123";

type SeedDoc = Record<string, Array<Record<string, unknown>>>;

function resolveValue(value: unknown, refs: Map<string, string>): unknown {
  if (typeof value === "string" && value.startsWith("$") && refs.has(value.slice(1))) {
    return refs.get(value.slice(1));
  }
  if (Array.isArray(value)) return value.map((v) => resolveValue(v, refs));
  return value;
}

async function main() {
  const cb = createClient(BASE_URL, { authCollection: "_superusers" });
  await cb.auth.signIn.password({ identity: SUPERUSER_EMAIL, password: SUPERUSER_PASSWORD });

  const already = await cb.collection("users").first({ filter: 'email = "alice@example.com"' });
  if (already) {
    console.log("==> Seed data already present (alice@example.com exists) — skipping.");
    return;
  }

  const doc: SeedDoc = await Bun.file(new URL("../seed.json", import.meta.url)).json();
  const refs = new Map<string, string>();
  let created = 0;

  for (const [collectionName, records] of Object.entries(doc)) {
    for (const raw of records) {
      const { $id, ...fields } = raw as { $id?: string } & Record<string, unknown>;
      const resolved: Record<string, unknown> = {};
      for (const [key, value] of Object.entries(fields)) {
        resolved[key] = resolveValue(value, refs);
      }
      let record;
      try {
        record = await cb.collection(collectionName).create(resolved);
      } catch (err) {
        const detail = err instanceof CratebaseError ? JSON.stringify(err.response) : String(err);
        throw new Error(`seeding ${collectionName} (${JSON.stringify(resolved)}) failed: ${detail}`);
      }
      if ($id) refs.set($id, record.id);
      created += 1;
    }
    console.log(`==> Seeded ${records.length} ${collectionName} record(s).`);
  }

  console.log(`==> Done — ${created} record(s) created.`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
