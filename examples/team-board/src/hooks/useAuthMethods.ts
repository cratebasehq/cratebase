// `GET /api/collections/users/auth-methods` lists which auth methods are
// actually configured — `oauth2.providers` only includes providers with a
// real client id/secret set (crates/server/src/routes/auth.rs), so this
// is what the auth screen uses to decide whether to render any OAuth
// buttons at all, rather than showing them unconditionally.
import { useEffect, useState } from "react";
import type { AuthMethodsList } from "@cratebase/client";
import { cb } from "../cratebase.js";

export function useAuthMethods() {
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

  return methods;
}
