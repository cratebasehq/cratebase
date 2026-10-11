import { createServerFn } from "@tanstack/react-start";
import { CratebaseError } from "@cratebase/client";
import { cb } from "~/lib/cratebase";

export interface SessionUser {
  id: string;
  email: string;
  name: string;
}

interface Credentials {
  email: string;
  password: string;
}

function readCredentials(data: unknown): Credentials {
  const { email, password } = (data ?? {}) as Partial<Credentials>;
  if (typeof email !== "string" || typeof password !== "string" || !email || !password) {
    throw new Error("Email and password are required.");
  }
  return { email: email.trim(), password };
}

/** The signed-in user, or `null`. Safe to call on every page load (cached per session token). */
export const getUserFn = createServerFn({ method: "GET" })
  .middleware([cb.optionalAuthMiddleware])
  .handler(async ({ context }): Promise<SessionUser | null> => {
    const user = context.user;
    return user ? { id: user.id, email: String(user.email ?? ""), name: String(user.name ?? "") } : null;
  });

export const signInFn = createServerFn({ method: "POST" })
  .validator(readCredentials)
  .handler(async ({ data }) => {
    try {
      // Saving the token fires `onChange`, which writes the HttpOnly session cookie.
      await cb.client().auth.signIn.password({ identity: data.email, password: data.password });
    } catch (err) {
      if (err instanceof CratebaseError && (err.status === 400 || err.status === 401)) {
        throw new Error("Invalid email or password.");
      }
      throw err;
    }
    return { ok: true as const };
  });

export const signUpFn = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ ...readCredentials(data), name: String((data as { name?: unknown }).name ?? "").trim() }))
  .handler(async ({ data }) => {
    try {
      // Creates the account, then signs in (which sets the session cookie).
      await cb.client().auth.signUp({
        email: data.email,
        password: data.password,
        passwordConfirm: data.password,
        name: data.name,
      });
    } catch (err) {
      if (err instanceof CratebaseError && err.status === 400) {
        throw new Error("Couldn't create that account — is the email already registered, and the password 8+ characters?");
      }
      throw err;
    }
    return { ok: true as const };
  });

export const signOutFn = createServerFn({ method: "POST" }).handler(async () => {
  cb.clearSession();
  return { ok: true as const };
});
