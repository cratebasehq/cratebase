"use client";

import { useEffect, type ReactNode } from "react";
import { useRouter } from "next/navigation";
import { useAuth } from "@/lib/cratebase";

export default function DashboardLayout({ children }: { children: ReactNode }) {
  const { isValid, isLoading } = useAuth();
  const router = useRouter();

  useEffect(() => {
    if (!isLoading && !isValid) router.replace("/sign-in");
  }, [isLoading, isValid, router]);

  if (isLoading || !isValid) {
    return <div className="flex min-h-screen items-center justify-center text-sm text-black/50">Loading…</div>;
  }

  return <>{children}</>;
}
