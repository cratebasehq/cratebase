import { useCallback } from "react";
import { useAuth as useCratebaseAuth } from "@cratebase/react";
import { cb } from "../cratebase";

export interface AuthUser {
  id: string;
  email: string;
}

function recordToUser(record: { id: string; email?: unknown } | null): AuthUser | null {
  if (!record) return null;
  return { id: record.id, email: typeof record.email === "string" ? record.email : "" };
}

/** Thin adapter over `@cratebase/react`'s `useAuth`, keeping this app's
 * own small `{ user, register, login, logout }` shape so
 * `App.tsx`/`AuthScreen.tsx` don't need to change: `@cratebase/react`'s
 * `AuthState` carries the raw `RecordModel` plus `isValid`/`isSuperuser`/
 * `isLoading`, none of which this board's UI needs. Mirrors the todo
 * example's auth pattern: `signUp` + `signIn.password` for register,
 * `signIn.password` alone for sign in. */
export function useAuth() {
  const { user: record, signIn, signOut } = useCratebaseAuth(cb);
  const user = recordToUser(record);

  const register = useCallback(async (email: string, password: string, passwordConfirm: string) => {
    await cb.auth.signUp({ email, password, passwordConfirm });
  }, []);

  const login = useCallback(
    async (email: string, password: string) => {
      await signIn.password({ identity: email, password });
    },
    [signIn],
  );

  const logout = useCallback(() => {
    cb.realtime.stop();
    void signOut();
  }, [signOut]);

  return { user, register, login, logout };
}
