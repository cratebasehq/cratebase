"use client";

import { useEffect, type ReactNode } from "react";
import { useRouter } from "next/navigation";
import { useAuth } from "@/lib/cratebase";

// Client-side guard: Cratebase's auth lives in localStorage, so the server
// can't see it. The data itself is protected server-side by collection rules.
export default function DashboardLayout({ children }: { children: ReactNode }) {
  const { isValid, isLoading } = useAuth();
  const router = useRouter();

  useEffect(() => {
    if (!isLoading && !isValid) router.replace("/sign-in");
  }, [isLoading, isValid, router]);

  if (isLoading || !isValid) {
    return (
      <div className="muted flex min-h-screen items-center justify-center text-sm" role="status">
        Loading…
      </div>
    );
  }

  return <>{children}</>;
}
