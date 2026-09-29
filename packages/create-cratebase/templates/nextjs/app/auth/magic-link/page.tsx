"use client";

import { useEffect } from "react";
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
    <div className="mx-auto mt-24 max-w-sm text-center">
      {status === "pending" && <p>Signing you in…</p>}
      {status === "error" && (
        <p className="text-red-500">That link didn't work{error ? `: ${String(error)}` : "."}</p>
      )}
    </div>
  );
}
