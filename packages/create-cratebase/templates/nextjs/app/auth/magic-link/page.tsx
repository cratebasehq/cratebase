"use client";

import { useEffect } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useMagicLinkCallback } from "@/lib/cratebase";

// Mounted at whatever URL a magic-link email points at (the `redirectUrl`
// passed to `cb.auth.magicLink.request`). `useMagicLinkCallback` reads the
// `?token=` query param, signs in, and strips the param from the URL on
// success — see sdk/js/react/README.md.
export default function MagicLinkCallbackPage() {
  const router = useRouter();
  const { status, error } = useMagicLinkCallback({
    onSuccess: () => router.replace("/dashboard"),
  });

  useEffect(() => {
    if (status === "none") router.replace("/sign-in");
  }, [status, router]);

  return (
    <main className="mx-auto mt-24 max-w-sm text-center" aria-live="polite">
      {status === "pending" && <p className="muted">Signing you in…</p>}
      {status === "error" && (
        <>
          <p role="alert" className="error-text">
            That link didn't work{error instanceof Error ? `: ${error.message}` : "."}
          </p>
          <Link href="/sign-in" className="btn-secondary mt-4">
            Back to sign in
          </Link>
        </>
      )}
    </main>
  );
}
