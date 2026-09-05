import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsFunctionsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/functions",
  component: lazyRouteComponent(() => import("@/components/settings/functions-page"), "FunctionsPage"),
});
