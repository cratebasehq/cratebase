/** Subscribes to a `CratebaseClient`'s auth state (`client.auth`, backed
 * by its `AuthStore`). `AuthStore.onChange` already fires synchronously on
 * every `save`/`clear` and its getters (`record`/`isValid`/`isSuperuser`)
 * are plain synchronous reads, which is exactly the shape
 * `useSyncExternalStore` wants — no polling, no `useState`+`useEffect`
 * race between the initial render and the first `onChange` firing.
 *
 * Two calling conventions: `useAuth(client)` (explicit) or `useAuth()`
 * inside a `<CratebaseProvider>` (reads from context) — see
 * {@link useResolvedClient}. */

import { useCallback, useRef, useSyncExternalStore } from "react";
import type { AuthNamespace, CratebaseClient, RecordModel } from "@cratebase/client";
import { useResolvedClient } from "./context.js";

export interface AuthState<U extends RecordModel = RecordModel> {
  /** `client.auth.record`, narrowed to `U` for callers with a typed users
   * collection. `null` when signed out. */
  user: U | null;
  token: string;
  isValid: boolean;
  isSuperuser: boolean;
  /** `true` only for the SSR/first-render snapshot, before this hook has
   * had a chance to read the client's (synchronous) auth store on the
   * client. Real auth stores (`LocalAuthStore`/`MemoryAuthStore`) hydrate
   * synchronously in their constructor, so this is `false` on every
   * client-side render — it exists purely so a server-rendered and a
   * first-hydrated-client render agree on what to show before the real
   * value is known, per `useSyncExternalStore`'s SSR contract. */
  isLoading: boolean;
  /** `client.auth.signIn` — `password`/`otp`/`code`/`social` sign-in
   * methods, forwarded as-is. */
  signIn: AuthNamespace["signIn"];
  /** `client.auth.signOut`, forwarded as-is. */
  signOut: AuthNamespace["signOut"];
}

const SERVER_SNAPSHOT: AuthState<any> = {
  user: null,
  token: "",
  isValid: false,
  isSuperuser: false,
  isLoading: true,
  signIn: undefined as unknown as AuthNamespace["signIn"],
  signOut: undefined as unknown as AuthNamespace["signOut"],
};

/** Live auth state, re-rendering on sign-in/sign-out/token refresh —
 * including changes made from outside React (another tab via
 * `LocalAuthStore`'s `storage` listener, or a direct
 * `client.auth.signOut()` call). `signIn`/`signOut` are convenience
 * pass-throughs to `client.auth.signIn`/`client.auth.signOut`. */
export function useAuth<U extends RecordModel = RecordModel>(client: CratebaseClient<any>): AuthState<U>;
export function useAuth<U extends RecordModel = RecordModel>(): AuthState<U>;
export function useAuth<U extends RecordModel = RecordModel>(client?: CratebaseClient<any>): AuthState<U> {
  const resolved = useResolvedClient(client);
  const lastSnapshot = useRef<AuthState<U> | null>(null);

  const getSnapshot = useCallback((): AuthState<U> => {
    const next: AuthState<U> = {
      user: resolved.auth.record as U | null,
      token: resolved.auth.token,
      isValid: resolved.auth.isValid,
      isSuperuser: resolved.auth.isSuperuser,
      isLoading: false,
      signIn: resolved.auth.signIn,
      signOut: resolved.auth.signOut.bind(resolved.auth),
    };
    const prev = lastSnapshot.current;
    if (
      prev &&
      !prev.isLoading &&
      prev.token === next.token &&
      prev.user === next.user &&
      prev.isValid === next.isValid &&
      prev.isSuperuser === next.isSuperuser
    ) {
      return prev;
    }
    lastSnapshot.current = next;
    return next;
  }, [resolved]);

  const getServerSnapshot = useCallback((): AuthState<U> => SERVER_SNAPSHOT, []);

  const subscribe = useCallback(
    (onStoreChange: () => void) => resolved.auth.onChange(() => onStoreChange()),
    [resolved],
  );

  return useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
