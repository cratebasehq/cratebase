import { createRoute } from "@tanstack/react-router";
import { DashboardHome } from "@/components/dashboard/dashboard-home";
import { appRoute } from "@/routes/app";

export const dashboardIndexRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/",
  component: DashboardHome,
});
