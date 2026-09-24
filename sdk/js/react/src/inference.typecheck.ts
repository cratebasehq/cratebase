/** Type-only inference checks for `createCratebaseHooks<Schema>()`,
 * checked with `tsc -p tsconfig.typecheck.json` (folded into `package.json`'s
 * `typecheck` script) — this file is never run, only compiled. A
 * `// @ts-expect-error` that stops being an actual error fails the
 * build (TypeScript reports "Unused '@ts-expect-error' directive"), so
 * this file doubles as a regression test for the inference itself, not
 * just a demonstration of it. */

import { createCratebaseHooks } from "./createHooks.js";

// A `type` alias, not an `interface` — TypeScript only infers an object
// type's implicit index signature (what lets it satisfy `AnySchema`'s
// `Record<string, Record<string, unknown>>` constraint) for a `type`
// alias, never for a named `interface`. `createCratebaseHooks<Schema>()`
// (and `createClient<Schema>()`, which the same constraint comes from)
// needs a `type`; see this package's README for the full explanation.
type TestSchema = {
  posts: { title: string; published: boolean; views: number };
  comments: { body: string; postId: string };
};

const { useRecords, useRecord, useMutation, useInfiniteRecords } = createCratebaseHooks<TestSchema>();

function Consumer() {
  const { records } = useRecords("posts", { sort: "-views", filter: "published = true" });
  const post = records[0];
  const title: string = post?.title ?? "";
  const published: boolean = post?.published ?? false;
  // `RecordModel`'s own fields (`id`, `collectionName`, ...) are still
  // present alongside the schema's own — the intersection this package's
  // hooks form under the hood (`S[K] & RecordModel`).
  const id: string = post?.id ?? "";

  // @ts-expect-error "postz" is not a key of TestSchema — every hook
  // this factory returns only accepts a real collection name.
  useRecords("postz");

  // @ts-expect-error `title` is a `string` on `TestSchema["posts"]`, not
  // assignable to `number`.
  const wrongFieldType: number = post!.title;

  const { record } = useRecord("comments", "abc123");
  const body: string = record?.body ?? "";
  // @ts-expect-error `postId` is a `comments` field typed `string`; a
  // `posts` record has no such field constraint, so this cross-checks
  // that `useRecord`'s second collection isn't accidentally widened to
  // `posts`'s shape.
  const crossCollectionTypo: string = post!.postId;

  const mutation = useMutation("posts");
  // @ts-expect-error `published` must be a `boolean`, not a `string`.
  void mutation.create({ published: "yes" });

  const infinite = useInfiniteRecords("posts", { sort: "-views" });

  return { title, published, id, body, wrongFieldType, crossCollectionTypo, records: infinite.records };
}

void Consumer;
