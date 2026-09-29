import type { ReactNode } from "react";
import { Navigate } from "react-router-dom";
import { useAuth } from "@/lib/cratebase";

export function RequireAuth({ children }: { children: ReactNode }) {
  const { isValid, isLoading } = useAuth();

  if (isLoading) {
    return <div className="flex min-h-screen items-center justify-center text-sm text-black/50">Loading…</div>;
  }
  if (!isValid) return <Navigate to="/sign-in" replace />;

  return <>{children}</>;
}
