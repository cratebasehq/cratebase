import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { WebhooksPage } from "@/components/settings/webhooks-page";

export const settingsWebhooksRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/webhooks",
  component: WebhooksPage,
});
