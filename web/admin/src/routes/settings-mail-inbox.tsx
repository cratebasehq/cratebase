import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type MailInboxSearch = {
  id?: string;
};

export const settingsMailInboxRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/mail-inbox",
  validateSearch: (search: Record<string, unknown>): MailInboxSearch => ({
    id: typeof search.id === "string" && search.id.length > 0 ? search.id : undefined,
  }),
  component: lazyRouteComponent(() => import("@/components/settings/mail-inbox-page"), "MailInboxPage"),
});
