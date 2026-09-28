import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsExtensionsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/extensions",
  component: lazyRouteComponent(() => import("@/components/settings/db-extensions-page"), "DbExtensionsPage"),
});
