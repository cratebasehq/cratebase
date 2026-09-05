import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export const settingsWebhooksRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/webhooks",
  component: lazyRouteComponent(() => import("@/components/settings/webhooks-page"), "WebhooksPage"),
});
