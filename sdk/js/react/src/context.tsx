/** `CratebaseProvider`/`useCratebase()` — an optional context so a React
 * tree doesn't have to thread a `CratebaseClient` instance through every
 * hook call by hand. Every hook in this package still accepts an
 * explicit client as an alternate, backward-compatible calling
 * convention (`useRecords(client, "posts")` as well as
 * `useRecords("posts")` inside a `<CratebaseProvider>`) — see
 * {@link useResolvedClient}, the shared seam every hook resolves its
 * client through.
 *
 * No `window`/`document` access happens here or at import time, so this
 * module is safe to import from a Next.js Server Component file; the
 * provider itself still needs a `"use client"` boundary somewhere above
 * it, same as any other context provider; see the docs page. */

import { createContext, useContext, type ReactNode } from "react";
import type { AnySchema, CratebaseClient } from "@cratebase/client";

const CratebaseContext = createContext<CratebaseClient<any> | null>(null);

export interface CratebaseProviderProps<S extends AnySchema = AnySchema> {
  client: CratebaseClient<S>;
  children?: ReactNode;
}

/** Makes `client` available to every hook in this package for the
 * subtree below, so they can be called without passing it explicitly
 * (`useRecords("posts")` instead of `useRecords(client, "posts")`). */
export function CratebaseProvider<S extends AnySchema = AnySchema>({
  client,
  children,
}: CratebaseProviderProps<S>) {
  return <CratebaseContext.Provider value={client}>{children}</CratebaseContext.Provider>;
}

/** Reads the `CratebaseClient` provided by the nearest `CratebaseProvider`.
 * Throws outside one — there is no sensible default client to fall back
 * to. Pass `S` explicitly to get a typed `collection()` back, or use
 * {@link createCratebaseHooks} for hooks that already know `S` and infer
 * collection record types from a collection name. */
export function useCratebase<S extends AnySchema = AnySchema>(): CratebaseClient<S> {
  const client = useContext(CratebaseContext);
  if (!client) {
    throw new Error(
      "useCratebase() was called outside a <CratebaseProvider>. Wrap your app (or the part of it " +
        "that uses Cratebase hooks) in <CratebaseProvider client={cb}>...</CratebaseProvider>.",
    );
  }
  return client as CratebaseClient<S>;
}

/** @internal Shared client-resolution seam for every hook in this
 * package: resolves to `explicit` when given (the legacy
 * `useX(client, ...)` calling convention) or to context (the new
 * `useX(...)` convention, used inside a `CratebaseProvider`).
 * `useContext` is always called, unconditionally, so branching on its
 * result afterwards never violates the rules of hooks. */
export function useResolvedClient(explicit: CratebaseClient<any> | undefined): CratebaseClient<any> {
  const contextClient = useContext(CratebaseContext);
  const client = explicit ?? contextClient;
  if (!client) {
    throw new Error(
      "No CratebaseClient available: pass one as this hook's first argument, or wrap your app in " +
        "<CratebaseProvider client={cb}>...</CratebaseProvider> and call the hook without one.",
    );
  }
  return client;
}
