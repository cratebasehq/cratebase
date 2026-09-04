import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { FileManagerPage } from "@/components/settings/file-manager-page";

export const settingsFileManagerRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/file-manager",
  component: FileManagerPage,
});
