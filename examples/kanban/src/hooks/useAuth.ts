import { useCallback, useEffect, useState } from "react";
import { cb } from "../cratebase";

export interface AuthUser {
  id: string;
  email: string;
}

function recordToUser(record: { id: string; email?: unknown } | null): AuthUser | null {
  if (!record) return null;
  return { id: record.id, email: typeof record.email === "string" ? record.email : "" };
}

/** Mirrors the todo example's auth pattern: `signUp` + `signIn.password`
 * for register, `signIn.password` alone for sign in. The SDK's
 * `AuthStore` persists the token/record to localStorage and fires
 * `onChange`, so a reload while signed in skips straight to the board —
 * this hook just subscribes to that. */
export function useAuth() {
  const [user, setUser] = useState<AuthUser | null>(() => recordToUser(cb.auth.record));

  useEffect(() => {
    return cb.auth.onChange(() => {
      setUser(recordToUser(cb.auth.record));
    });
  }, []);

  const register = useCallback(async (email: string, password: string, passwordConfirm: string) => {
    await cb.auth.signUp({ email, password, passwordConfirm });
  }, []);

  const login = useCallback(async (email: string, password: string) => {
    await cb.auth.signIn.password({ identity: email, password });
  }, []);

  const logout = useCallback(() => {
    cb.realtime.stop();
    cb.auth.signOut();
  }, []);

  return { user, register, login, logout };
}
