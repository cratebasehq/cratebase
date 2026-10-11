import { useEffect } from "react";
import { Link, useNavigate } from "react-router-dom";
import { useMagicLinkCallback } from "@/lib/cratebase";

// Mounted at whatever URL a magic-link email points at (the `redirectUrl`
// passed to `cb.auth.magicLink.request`). `useMagicLinkCallback` reads the
// `?token=` query param, signs in, and strips the param from the URL on
// success — see sdk/js/react/README.md.
export function MagicLinkCallback() {
  const navigate = useNavigate();
  const { status, error } = useMagicLinkCallback({
    onSuccess: () => navigate("/dashboard", { replace: true }),
  });

  useEffect(() => {
    if (status === "none") navigate("/sign-in", { replace: true });
  }, [status, navigate]);

  return (
    <main className="mx-auto mt-24 max-w-sm text-center" aria-live="polite">
      {status === "pending" && <p className="muted">Signing you in…</p>}
      {status === "error" && (
        <>
          <p role="alert" className="error-text">
            That link didn't work{error instanceof Error ? `: ${error.message}` : "."}
          </p>
          <Link to="/sign-in" className="btn-secondary mt-4">
            Back to sign in
          </Link>
        </>
      )}
    </main>
  );
}
