import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsPushRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/push",
  component: lazyRouteComponent(() => import("@/components/settings/push-page"), "PushPage"),
});
