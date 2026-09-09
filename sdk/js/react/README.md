# @cratebase/react

First-party React hooks for [`@cratebase/client`](https://www.npmjs.com/package/@cratebase/client),
Cratebase's typed TypeScript SDK. This package is a thin, optional layer on top of the SDK: it
does not wrap or replace anything, it just exposes `CratebaseClient`'s existing primitives
(`AuthStore.onChange`, `CollectionService.subscribe`) as idiomatic hooks so a React app doesn't
have to hand-roll the same `useEffect`/subscribe/unsubscribe/id-merge pattern every screen ends up
needing — see `examples/kanban/src/hooks/{useAuth,useCards,usePresence}.ts` for what that
hand-rolled version looks like.

A project that only uses `@cratebase/client` directly (server code, a non-React frontend, a React
app that prefers its own data layer) never installs `@cratebase/react` and pays zero cost. A React
app that wants these hooks adds one dependency on top of the client it already has, and passes
that existing client instance in — every hook here takes a `CratebaseClient` as its first
argument; none of them construct their own client or duplicate auth/session handling.

```
bun add @cratebase/client @cratebase/react react
```

`@cratebase/client` (pinned to `^0.1.0`, the version this package was built and verified against)
and `react` (`^18 || ^19`) are peer dependencies — you bring your own `CratebaseClient` instance,
already pointed at your server however your app normally configures it.

```ts
import { createClient } from "@cratebase/client";

export const cb = createClient("http://127.0.0.1:8090");
```

## `useAuth` — live auth state

Subscribes to `client.auth` (backed by its `AuthStore`) via `useSyncExternalStore`, re-rendering
on sign-in, sign-out, token refresh, or an out-of-band change (another browser tab writing to the
same `LocalAuthStore`, or a direct `client.auth.signOut()` call from outside React). It only
observes state — call `client.auth.signIn.password(...)`, `client.auth.signUp(...)`, or
`client.auth.signOut()` directly and let the hook pick up the result.

```tsx
import { useAuth } from "@cratebase/react";
import { cb } from "./cratebase";

function Nav() {
  const { user, isValid, isSuperuser } = useAuth(cb);

  if (!isValid) return <button onClick={() => cb.auth.signIn.password({ identity, password })}>Sign in</button>;
  return (
    <span>
      {user?.email} {isSuperuser && "(admin)"}
      <button onClick={() => cb.auth.signOut()}>Sign out</button>
    </span>
  );
}
```

## `useRecords` — live list

Fetches every record in a collection with `fullList`, then keeps the list in sync with a `"*"`
realtime subscription, merging `create`/`update`/`delete` events by id — the same pattern
`examples/kanban/src/hooks/useCards.ts` hand-rolls. Accepts the same options as
`CollectionService.list`/`fullList` (`filter`, `sort`, `expand`, `fields`), plus `enabled` to skip
the fetch/subscribe entirely (e.g. while a filter value isn't known yet) and `subscribe` to scope
the realtime subscription differently from the initial fetch.

```tsx
import { useRecords } from "@cratebase/react";
import { cb } from "./cratebase";

function PostList() {
  const { records: posts, loading, error } = useRecords(cb, "posts", { sort: "-created", filter: 'published = true' });

  if (loading) return <p>Loading…</p>;
  if (error) return <p>Failed to load posts.</p>;
  return (
    <ul>
      {posts.map((post) => (
        <li key={post.id}>{post.title}</li>
      ))}
    </ul>
  );
}
```

Writes go through `client.collection("posts").create()`/`.update()`/`.delete()` directly (this
hook does not wrap writes); they land in `records` once their own realtime event arrives, merged
against whatever is already there by id.

## `useRecord` — live single record

The single-record variant of `useRecords`: fetches once with `one()`, then subscribes to that
record's own topic rather than `"*"` so the server only pushes events for this one row.

```tsx
import { useRecord } from "@cratebase/react";
import { cb } from "./cratebase";

function PostDetail({ postId }: { postId: string }) {
  const { record: post, loading, deleted } = useRecord(cb, "posts", postId);

  if (loading) return <p>Loading…</p>;
  if (deleted) return <p>This post was deleted.</p>;
  return <h1>{post?.title}</h1>;
}
```

## `useSubscription` — raw realtime events

A thin, stateless primitive for callers who want raw realtime events without `useRecords`'/
`useRecord`'s list-merging opinion — wraps `CollectionService.subscribe` with effect cleanup and a
stable callback ref, so passing a new inline arrow function every render does not force a
resubscribe.

```tsx
import { useSubscription } from "@cratebase/react";
import { cb } from "./cratebase";

function ActivityToast() {
  useSubscription(cb, "posts", "*", (event) => {
    if (event.action === "create") toast(`New post: ${event.record.title}`);
  });
  return null;
}
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

```
bun install
bunx tsc --noEmit   # typecheck
bun run build        # emit dist/
```
