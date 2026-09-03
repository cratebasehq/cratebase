import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { BackupsPage } from "@/components/settings/backups-page";

export const settingsBackupsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/backups",
  component: BackupsPage,
});
