import { useEffect, useState } from "react";
import { useMagicLinkCallback } from "@cratebase/react";
import { cb, describeError, useAuth } from "./cratebase.js";
import { AuthScreen } from "./components/AuthScreen.js";
import { Workspace } from "./components/Workspace.js";

export function App() {
  const { user, isLoading } = useAuth();
  const [magicLinkError, setMagicLinkError] = useState("");

  // Completes an OAuth2 redirect (`?cb_error=`/`?cb_mfa=` on return from
  // the provider) if this load is one — a no-op otherwise.
  useEffect(() => {
    if (window.location.search.includes("cb_")) {
      cb.auth.completeSocial().catch(() => {});
    }
  }, []);

  // `scripts/setup.sh` points `settings.meta.appURL` at this dev
  // server, so the link `cb.auth.magicLink.request(...)` emails lands
  // back here with a `?token=` — this hook reads it, signs in, and
  // strips the param, regardless of which path it lands on (this app
  // has no client router, so `/auth/magic-link` just serves the same
  // `index.html` Vite always does).
  const { status: magicLinkStatus } = useMagicLinkCallback(cb, {
    onError: (err) => setMagicLinkError(describeError(err)),
  });

  if (isLoading) return null;
  if (magicLinkStatus === "pending") return <CenteredMessage text="Signing you in…" />;
  if (!user) return <AuthScreen magicLinkError={magicLinkError} />;
  return <Workspace />;
}

function CenteredMessage({ text }: { text: string }) {
  return (
    <div className="flex min-h-screen items-center justify-center text-sm text-ink/60 dark:text-paper/60">
      {text}
    </div>
  );
}
