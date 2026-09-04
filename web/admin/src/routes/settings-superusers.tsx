import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsSuperusersRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/superusers",
  component: lazyRouteComponent(() => import("@/components/settings/superusers-page"), "SuperusersPage"),
});
