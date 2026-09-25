# @cratebase/react

First-party React hooks for [`@cratebase/client`](https://www.npmjs.com/package/@cratebase/client),
Cratebase's typed TypeScript SDK. This package is a thin, optional layer on top of the SDK: it does
not wrap or replace anything, it exposes `CratebaseClient`'s existing primitives
(`AuthStore.onChange`, `CollectionService.subscribe`, `presence.track`) as idiomatic hooks so a
React app doesn't have to hand-roll the same `useEffect`/subscribe/unsubscribe/id-merge pattern
every screen ends up needing.

A project that only uses `@cratebase/client` directly (server code, a non-React frontend, a React
app that prefers its own data layer) never installs `@cratebase/react` and pays zero cost.

```
bun add @cratebase/client @cratebase/react react
```

`@cratebase/client` (pinned to `^0.1.0`, the version this package was built and verified against)
and `react` (`^18 || ^19`) are peer dependencies.

## Two ways to use these hooks

**Explicit client** — every hook takes a `CratebaseClient` as its first argument, no provider
needed:

```ts
import { createClient } from "@cratebase/client";

export const cb = createClient("http://127.0.0.1:8090");
```

```tsx
import { useRecords } from "@cratebase/react";
import { cb } from "./cratebase";

function PostList() {
  const { records: posts, loading, error } = useRecords(cb, "posts", { sort: "-created" });
  if (loading) return <p>Loading…</p>;
  return <ul>{posts.map((p) => <li key={p.id}>{p.title}</li>)}</ul>;
}
```

**`CratebaseProvider`** — wrap your app once, then call the same hooks without a client:

```tsx
import { CratebaseProvider } from "@cratebase/react";
import { cb } from "./cratebase";

export function App({ children }: { children: React.ReactNode }) {
  return <CratebaseProvider client={cb}>{children}</CratebaseProvider>;
}
```

```tsx
import { useRecords } from "@cratebase/react";

function PostList() {
  const { records: posts } = useRecords("posts", { sort: "-created" });
  // ...
}
```

Every hook resolves its client from whichever it's given — an explicit first argument, falling
back to `CratebaseProvider` context — via a shared internal `useResolvedClient` (`useContext` is
always called, so branching on its result afterwards never breaks the rules of hooks). Both styles
work in the same app.

> **Next.js App Router**: `CratebaseProvider` and every hook here use React context and
> `useSyncExternalStore`, so anything that renders them needs a `"use client"` boundary — typically
> a small `<Providers>` client component wrapping `CratebaseProvider`, rendered from a server
> layout. Nothing in this package touches `window`/`document` at import time.

## Typed collections: `createCratebaseHooks<Schema>()`

Generate a `Schema` with [`@cratebase/schema-codegen`](../../../tools/schema-codegen) (the same one
`createClient<Schema>()` takes) and pass it once to `createCratebaseHooks` for
collection-name-to-record-type inference on every hook:

```ts
// cratebase.ts
import { createClient } from "@cratebase/client";
import { createCratebaseHooks } from "@cratebase/react";
import type { Schema } from "./cratebase-types";

export const cb = createClient<Schema>("http://localhost:8090");
export const { useRecords, useRecord, useInfiniteRecords, useMutation, useAuth, useCratebase } =
  createCratebaseHooks<Schema>();
```

```tsx
import { useRecords } from "./cratebase";

const { records: posts } = useRecords("posts", { sort: "-created" }); // typed from Schema["posts"]
```

**Why a factory, not module augmentation**: augmenting a global interface only supports one schema
per process — awkward for tests (a fixture schema would leak across test files) and for any app
with more than one client/schema. A factory keeps the schema an ordinary type parameter, at the
cost of one extra call site, the same trade `zustand`'s and `jotai`'s typed-store factories make.
The hooks it returns still read their client from `CratebaseProvider` context (or take one
explicitly) — this only narrows *types*.

> **`type`, not `interface`**: `Schema` must be `type Schema = { ... }`, not `interface Schema {
> ... }`. TypeScript only infers an object type's implicit index signature — what lets it satisfy
> `AnySchema`'s `Record<string, Record<string, unknown>>` constraint — for a `type` alias, never a
> named `interface`. This applies to `createClient<Schema>()` too.

## `useAuth` — live auth state

Subscribes to `client.auth` via `useSyncExternalStore`, re-rendering on sign-in, sign-out, token
refresh, or an out-of-band change (another tab writing to the same `LocalAuthStore`, or a direct
`client.auth.signOut()` call). `signIn`/`signOut` are convenience pass-throughs to
`client.auth.signIn`/`client.auth.signOut`; `isLoading` is `true` only for the SSR/first-render
snapshot (every real auth store hydrates synchronously, so it's `false` on any client render).

```tsx
import { useAuth } from "@cratebase/react";
import { cb } from "./cratebase";

function Nav() {
  const { user, isValid, isSuperuser, signIn, signOut } = useAuth(cb);
  if (!isValid) return <button onClick={() => signIn.password({ identity, password })}>Sign in</button>;
  return <span>{user?.email} {isSuperuser && "(admin)"} <button onClick={() => signOut()}>Sign out</button></span>;
}
```

## `useRecords` — live list

Fetches with `fullList` (no `page`/`perPage`) or a single `list()` page (either given), then — when
`realtime` is enabled (the default) — refetches on every matching realtime event, debounced to
coalesce a burst of writes. Refetch, not local create/update/delete patching: a patched list has to
reimplement the server's own filter/sort/pagination semantics client-side to stay correct, which
quietly drifts; refetching keeps the list exactly what the server would return for the same query.

```tsx
import { useRecords } from "@cratebase/react";
import { cb } from "./cratebase";

function PostList() {
  const { records: posts, loading, error, page, totalPages } = useRecords(cb, "posts", {
    sort: "-created",
    filter: "published = true",
    page: 1,
    perPage: 20,
  });
  // ...
}
```

Writes go through `useMutation` or `client.collection("posts").create()`/`.update()`/`.delete()`
directly — this hook does not wrap writes.

## `useRecord` — live single record

The single-record variant of `useRecords`: fetches once with `one()`, then subscribes to that
record's own topic (so the server only pushes events for this one row) and applies the event
directly rather than refetching.

```tsx
import { useRecord } from "@cratebase/react";
import { cb } from "./cratebase";

function PostDetail({ postId }: { postId: string }) {
  const { record: post, loading, deleted } = useRecord(cb, "posts", postId);
  if (deleted) return <p>This post was deleted.</p>;
  return <h1>{post?.title}</h1>;
}
```

## `useInfiniteRecords` — load more

Accumulates pages into one growing `records` array; `loadMore()` fetches the next one. Realtime
defaults to `false` (unlike `useRecords`) — refetching from page 1 on every event would silently
reset however many pages the caller has already loaded. Pass `realtime: true` to opt in anyway.

```tsx
const { records, loading, loadingMore, hasMore, loadMore } = useInfiniteRecords(cb, "posts", { perPage: 20 });
```

## `useMutation` — create/update/remove with optimistic UI

`pending`/`error` state, plus an optional `{ apply, rollback }` pair per call for optimistic UI —
`apply()` runs before the request is sent, `rollback()` only if it throws. Decoupled from any
particular list's state; wire it up to whatever you're showing optimistically.

```tsx
const { create, update, remove, pending, error } = useMutation(cb, "posts");

await create(
  { title },
  { optimistic: { apply: () => setLocal((p) => [...p, draft]), rollback: () => setLocal((p) => p.filter((x) => x !== draft)) } },
);
```

## `usePresence` — who's online

Wraps `client.presence.track` (heartbeat + realtime + client-side staleness, no server-side
presence concept needed):

```tsx
const { online, loading } = usePresence(cb, "presence", { userRef: user.id });
```

## `useSubscription` — raw realtime events

A thin, stateless primitive for callers who want raw realtime events without `useRecords`'/
`useRecord`'s list-merging opinion.

```tsx
useSubscription(cb, "posts", "*", (event) => {
  if (event.action === "create") toast(`New post: ${event.record.title}`);
});
```

## Versioning

This package versions independently from the `cratebase` server binary and from
`@cratebase/client`. It's an optional React add-on with its own semver lifecycle — the
`@cratebase/client` peer dependency it targets, not the server release cadence, is what actually
constrains breaking changes here. Publishing is triggered by pushing a `react-v<version>` tag (see
`.github/workflows/publish-react.yml`), kept deliberately separate from the server's own `v*`
release tags and from `client-v*`/`extras-v*`, so a server or client release never forces an
unrelated hooks publish, and a hooks hotfix never waits on one.

## Development

This package is part of the root bun workspace (`sdk/js/client`, `sdk/js/react`, `sdk/js/extras`),
so `bun install` at the repo root sets it up too.

```
bun install
bun run --cwd sdk/js/react typecheck   # tsc --noEmit + the type-inference fixture
bun run --cwd sdk/js/react test        # bun test (happy-dom + @testing-library/react)
bun run --cwd sdk/js/react build       # emit dist/
```

`bun run sdk:check` from the repo root runs all three SDK packages' checks in one command (also
wired into CI).
