import { useCallback, useEffect, useState } from "react";
import { pb } from "../pocketbase";

export interface AuthUser {
  id: string;
  email: string;
}

function recordToUser(record: { id: string; email?: string } | null): AuthUser | null {
  if (!record) return null;
  return { id: record.id, email: record.email ?? "" };
}

/** Mirrors the todo example's auth pattern: `users.create` +
 * `authWithPassword` for register, `authWithPassword` alone for sign in.
 * The SDK's `AuthStore` persists the token/record to localStorage and
 * fires `onChange`, so a reload while signed in skips straight to the
 * board — this hook just subscribes to that. */
export function useAuth() {
  const [user, setUser] = useState<AuthUser | null>(() => recordToUser(pb.authStore.record as never));

  useEffect(() => {
    return pb.authStore.onChange(() => {
      setUser(recordToUser(pb.authStore.record as never));
    });
  }, []);

  const register = useCallback(async (email: string, password: string, passwordConfirm: string) => {
    await pb.collection("users").create({ email, password, passwordConfirm });
    await pb.collection("users").authWithPassword(email, password);
  }, []);

  const login = useCallback(async (email: string, password: string) => {
    await pb.collection("users").authWithPassword(email, password);
  }, []);

  const logout = useCallback(() => {
    pb.realtime.unsubscribe();
    pb.authStore.clear();
  }, []);

  return { user, register, login, logout };
}
