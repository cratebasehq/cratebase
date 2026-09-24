import { useEffect } from "react";
import { cb, useAuth } from "./cratebase.js";
import { AuthScreen } from "./components/AuthScreen.js";
import { Workspace } from "./components/Workspace.js";

export function App() {
  const { user, isLoading } = useAuth();

  // Completes an OAuth2 redirect (`?cb_error=`/`?cb_mfa=` on return from
  // the provider) if this load is one — a no-op otherwise.
  useEffect(() => {
    if (window.location.search.includes("cb_")) {
      cb.auth.completeSocial().catch(() => {});
    }
  }, []);

  if (isLoading) return null;
  if (!user) return <AuthScreen />;
  return <Workspace />;
}
