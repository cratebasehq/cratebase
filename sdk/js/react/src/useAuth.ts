/** Subscribes to a `CratebaseClient`'s auth state (`client.auth`, backed
 * by its `AuthStore`). `AuthStore.onChange` already fires synchronously on
 * every `save`/`clear` and its getters (`record`/`isValid`/`isSuperuser`)
 * are plain synchronous reads, which is exactly the shape
 * `useSyncExternalStore` wants — no polling, no `useState`+`useEffect`
 * race between the initial render and the first `onChange` firing. */

import { useCallback, useRef, useSyncExternalStore } from "react";
import type { CratebaseClient, RecordModel } from "@cratebase/client";

export interface AuthState<U extends RecordModel = RecordModel> {
  /** `client.auth.record`, narrowed to `U` for callers with a typed users
   * collection. `null` when signed out. */
  user: U | null;
  token: string;
  isValid: boolean;
  isSuperuser: boolean;
}

/** Live auth state for `client`, re-rendering on sign-in/sign-out/token
 * refresh — including changes made from outside React (another tab via
 * `LocalAuthStore`'s `storage` listener, or a direct `client.auth.signOut()`
 * call). Does not itself sign in/out; call `client.auth.signIn.password(...)`
 * etc. directly and let this hook observe the result. */
export function useAuth<U extends RecordModel = RecordModel>(client: CratebaseClient<any>): AuthState<U> {
  const lastSnapshot = useRef<AuthState<U> | null>(null);

  const getSnapshot = useCallback((): AuthState<U> => {
    const next: AuthState<U> = {
      user: client.auth.record as U | null,
      token: client.auth.token,
      isValid: client.auth.isValid,
      isSuperuser: client.auth.isSuperuser,
    };
    const prev = lastSnapshot.current;
    if (prev && prev.token === next.token && prev.user === next.user && prev.isValid === next.isValid && prev.isSuperuser === next.isSuperuser) {
      return prev;
    }
    lastSnapshot.current = next;
    return next;
  }, [client]);

  const subscribe = useCallback(
    (onStoreChange: () => void) => client.auth.onChange(() => onStoreChange()),
    [client],
  );

  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
