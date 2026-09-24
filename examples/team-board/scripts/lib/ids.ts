// Reimplements `cratebase_core::ids::collection_id` (`crates/core/src/ids.rs`)
// so this example can compute a collection's id *before* it exists —
// Cratebase derives every collection's id deterministically from its type
// and name (`pbc_<crc32(type + name)>`, standard IEEE CRC-32, matching
// PocketBase's own convention), rather than a random id. That means a
// `relation` field's required `collectionId` (see
// `crates/core/src/field.rs`'s `FieldKind::Relation`) can be computed
// entirely offline, with no server round-trip and no dependency-ordered
// "create, read back the id, patch the next collection" dance — see
// `scripts/gen-schema.ts`.
//
// Verified against the server's own doc comment
// (`crates/core/src/ids.rs:24-25`, "pbc_2279338944 for base _mfas"):
// `crc32("base" + "_mfas") === 2279338944` reproduced exactly by both
// Python's `zlib.crc32` and this implementation before this file was
// trusted for anything load-bearing.

const CRC32_POLY = 0xedb88320;

function crc32(input: string): number {
  const bytes = Buffer.from(input, "utf8");
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let i = 0; i < 8; i++) {
      crc = (crc >>> 1) ^ (CRC32_POLY & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

/** `type` is the collection's kind (`"base"`, `"auth"`, or `"view"`), not
 * a TypeScript type. Matches `crates/core/src/ids.rs::collection_id`. */
export function collectionId(type: "base" | "auth" | "view", name: string): string {
  return `pbc_${crc32(type + name)}`;
}
