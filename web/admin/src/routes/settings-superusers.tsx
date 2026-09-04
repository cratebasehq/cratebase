import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { SuperusersPage } from "@/components/settings/superusers-page";

export const settingsSuperusersRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/superusers",
  component: SuperusersPage,
});
