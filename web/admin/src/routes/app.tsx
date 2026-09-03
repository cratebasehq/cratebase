import { createRoute, redirect } from "@tanstack/react-router";
import { AppShell } from "@/components/layout/app-shell";
import { isLoggedIn } from "@/lib/api";
import { rootRoute } from "@/routes/root";

export const appRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: "_app",
  beforeLoad: () => {
    if (!isLoggedIn()) {
      throw redirect({ to: "/login" });
    }
  },
  component: AppShell,
});
