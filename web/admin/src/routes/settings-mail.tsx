import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { MailStoragePage } from "@/components/settings/mail-storage-page";

export const settingsMailRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/mail-storage",
  component: MailStoragePage,
});
