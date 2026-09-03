import { createRoute } from "@tanstack/react-router";
import { EmptyDashboard } from "@/components/layout/app-shell";
import { appRoute } from "@/routes/app";

export const dashboardIndexRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/",
  component: EmptyDashboard,
});
