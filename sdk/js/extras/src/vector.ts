/** `?nearestTo=` — application-side cosine-similarity ranking for a
 * `vector` field (see `FieldKind::Vector` in
 * `crates/core/src/field.rs` and `crate::embeddings` in
 * `crates/server/src/embeddings.rs`), exposed as a typed wrapper around
 * `pb.collection(name).getList()` rather than a hand-built query string.
 *
 * There is no native ANN index behind this (no `sqlite-vec`/`pgvector`
 * in this pass) — it fetches every row the collection's `listRule`/
 * `?filter=` would normally return (capped server-side at 20,000 rows),
 * ranks by cosine similarity, and truncates to `limit`. Fine for
 * app-scale collections (thousands to low millions of vectors), not a
 * substitute for a dedicated vector DB at massive scale.
 */

import type PocketBase from "pocketbase";
import type { ListResult, RecordListOptions, RecordModel } from "pocketbase";

/** Options for {@link nearestTo}. Everything `getList` already accepts
 * (`filter`, `expand`, `fields`, ...) still applies — `filter` narrows
 * the candidate set *before* ranking, the same way it would for a plain
 * list call. `sort`/`page`/`perPage` are accepted by the type but ignored
 * server-side: `?nearestTo=` always returns one page, ranked by
 * similarity, not by `sort`. */
export interface NearestToOptions extends Omit<RecordListOptions, "sort"> {
  /** How many ranked results to return. Mirrors the server's own
   * `nearestLimit` default of 20, clamped to the collection's normal
   * per-page ceiling. */
  limit?: number;
}

/** Rank `collectionIdOrName`'s rows by cosine similarity to `to` on
 * `field`, and return them exactly like a normal `getList` page (one
 * page, already truncated to `limit`).
 *
 * `to` is either the query vector itself (`number[]`, matching the
 * field's configured `dimensions`) or another record's id — whose own
 * `field` value becomes the query vector, subject to that record's own
 * `viewRule` (so this cannot be used to probe a hidden record's vector).
 *
 * ```ts
 * import PocketBase from "pocketbase";
 * import { nearestTo } from "@cratebase/extras";
 *
 * const pb = new PocketBase("http://127.0.0.1:8090");
 * await pb.collection("_superusers").authWithPassword(email, password);
 *
 * // by an explicit query vector
 * const byVector = await nearestTo(pb, "chunks", "embedding", [0.12, -0.4, 0.91], {
 *   limit: 5,
 *   filter: 'docId = "abc123"',
 * });
 *
 * // by "more like this record"
 * const similar = await nearestTo(pb, "chunks", "embedding", "REC_ID_HERE", { limit: 5 });
 * ```
 */
export async function nearestTo<T extends RecordModel = RecordModel>(
  pb: PocketBase,
  collectionIdOrName: string,
  field: string,
  to: number[] | string,
  options: NearestToOptions = {},
): Promise<ListResult<T>> {
  const { limit, ...rest } = options;
  const target = Array.isArray(to) ? to.join(",") : to;
  return pb.collection(collectionIdOrName).getList<T>(1, limit ?? 20, {
    ...rest,
    nearestTo: `${field}:${target}`,
    ...(limit !== undefined ? { nearestLimit: limit } : {}),
  });
}
