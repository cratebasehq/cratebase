"use client";

import { useEffect, useState } from "react";
import { cb } from "@/lib/cratebase";
import type { AuthMethodsList } from "@cratebase/client";

/**
 * Renders one button per OAuth2 provider actually configured on the server
 * (`GET /api/collections/users/auth-methods`) — nothing hardcoded, so this
 * renders empty (not broken) on a fresh install with no providers set up
 * yet. See https://cratebase.dev/docs/concepts/authentication/oauth2/.
 */
export function OAuthButtons() {
  const [methods, setMethods] = useState<AuthMethodsList | null>(null);

  useEffect(() => {
    let cancelled = false;
    cb.auth
      .methods()
      .then((m) => {
        if (!cancelled) setMethods(m);
      })
      .catch(() => {
        if (!cancelled) setMethods(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const providers = methods?.oauth2.enabled ? methods.oauth2.providers : [];
  if (providers.length === 0) return null;

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-3 text-xs text-black/40 dark:text-white/40">
        <div className="h-px flex-1 bg-current/20" />
        <span>or continue with</span>
        <div className="h-px flex-1 bg-current/20" />
      </div>
      <div className="grid grid-cols-2 gap-2">
        {providers.map((provider) => (
          <button
            key={provider.name}
            type="button"
            className="btn-secondary"
            onClick={() => cb.auth.signIn.social({ provider: provider.name })}
          >
            {provider.displayName}
          </button>
        ))}
      </div>
    </div>
  );
}
