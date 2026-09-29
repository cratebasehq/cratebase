import { useEffect } from "react";
import { useNavigate } from "react-router-dom";
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
    <div className="mx-auto mt-24 max-w-sm text-center">
      {status === "pending" && <p>Signing you in…</p>}
      {status === "error" && (
        <p className="text-red-500">That link didn't work{error ? `: ${String(error)}` : "."}</p>
      )}
    </div>
  );
}
