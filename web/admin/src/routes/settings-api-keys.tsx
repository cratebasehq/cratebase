import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { ApiKeysPage } from "@/components/settings/api-keys-page";

export const settingsApiKeysRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/api-keys",
  component: ApiKeysPage,
});
