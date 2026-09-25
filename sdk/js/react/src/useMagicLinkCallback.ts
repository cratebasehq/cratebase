/** Completes a magic-link sign-in from the current page's URL: on mount,
 * reads a `?token=` query param (see `getMagicLinkTokenFromUrl` in
 * `@cratebase/client`) and, if present, calls
 * `client.auth.signIn.magicLink({ token })`. Meant for the page
 * `authOptions.magicLink.urlTemplate` points at (or a custom
 * `redirectUrl`) — mount it there and it drives the sign-in itself, no
 * form needed. */

import { useEffect, useRef, useState } from "react";
import type { AuthResult, CratebaseClient } from "@cratebase/client";
import { getMagicLinkTokenFromUrl } from "@cratebase/client";
import { useResolvedClient } from "./context.js";

export interface UseMagicLinkCallbackOptions {
  /** Query param to read the token from. Defaults to `"token"`. */
  param?: string;
  /** Strip the token from the URL (via `history.replaceState`) once
   * handled, so a refresh doesn't try to redeem it again. Defaults to
   * `true`. */
  removeParam?: boolean;
  onSuccess?: (result: AuthResult) => void;
  onError?: (error: unknown) => void;
}

export type UseMagicLinkCallbackStatus = "idle" | "pending" | "success" | "error" | "none";

export interface UseMagicLinkCallbackResult {
  /** `"none"` when the URL carried no token — nothing to do. */
  status: UseMagicLinkCallbackStatus;
  error: unknown;
}

function isClientArg(value: unknown): value is CratebaseClient<any> {
  // Options never has an `auth` key, and every real (or test-fake)
  // client has an `auth` namespace — a cheaper, mock-friendly check than
  // requiring every internal field a real `CratebaseClient` happens to
  // carry (e.g. `transport`).
  return !!value && typeof value === "object" && "auth" in value;
}

export function useMagicLinkCallback(
  client: CratebaseClient<any>,
  options?: UseMagicLinkCallbackOptions,
): UseMagicLinkCallbackResult;
export function useMagicLinkCallback(options?: UseMagicLinkCallbackOptions): UseMagicLinkCallbackResult;
export function useMagicLinkCallback(
  clientOrOptions?: CratebaseClient<any> | UseMagicLinkCallbackOptions,
  maybeOptions?: UseMagicLinkCallbackOptions,
): UseMagicLinkCallbackResult {
  const explicitClient = isClientArg(clientOrOptions) ? clientOrOptions : undefined;
  const options = (isClientArg(clientOrOptions) ? maybeOptions : clientOrOptions) ?? {};
  const client = useResolvedClient(explicitClient);

  const { param = "token", removeParam = true, onSuccess, onError } = options;
  const [status, setStatus] = useState<UseMagicLinkCallbackStatus>("idle");
  const [error, setError] = useState<unknown>(null);
  const ran = useRef(false);

  useEffect(() => {
    if (ran.current) return;
    ran.current = true;

    const token = getMagicLinkTokenFromUrl(undefined, param);
    if (!token) {
      setStatus("none");
      return;
    }

    setStatus("pending");
    client.auth.signIn
      .magicLink({ token })
      .then((result) => {
        setStatus("success");
        if (removeParam && typeof window !== "undefined") {
          const url = new URL(window.location.href);
          url.searchParams.delete(param);
          window.history.replaceState({}, "", url.toString());
        }
        onSuccess?.(result);
      })
      .catch((err) => {
        setStatus("error");
        setError(err);
        onError?.(err);
      });
    // Runs once on mount by design (see `ran`) — a magic link is
    // single-use, so re-running this on a dependency change would just
    // fail the second time.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { status, error };
}
