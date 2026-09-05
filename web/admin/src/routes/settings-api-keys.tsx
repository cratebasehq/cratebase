import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsApiKeysRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/api-keys",
  component: lazyRouteComponent(() => import("@/components/settings/api-keys-page"), "ApiKeysPage"),
});
