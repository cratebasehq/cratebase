import { Cratebase } from "cratebase";

/** Single shared client for the whole dashboard. `authStore` persists the
 * superuser session to `localStorage` under this key so a page reload
 * doesn't require logging back in. */
export const cb = new Cratebase(import.meta.env.VITE_API_URL ?? "");

export function isLoggedIn(): boolean {
  return cb.authStore.isValid && cb.authStore.model !== null;
}
