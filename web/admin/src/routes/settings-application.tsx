import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsApplicationRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/application",
  component: lazyRouteComponent(() => import("@/components/settings/application-page"), "ApplicationPage"),
});
