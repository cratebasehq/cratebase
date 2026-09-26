import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type MailLogSearch = {
  page?: number;
  status?: "sent" | "failed" | "queued";
  template?: string;
};

export const settingsMailLogRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/mail-log",
  validateSearch: (search: Record<string, unknown>): MailLogSearch => ({
    page: typeof search.page === "number" && search.page > 1 ? search.page : undefined,
    status:
      search.status === "sent" || search.status === "failed" || search.status === "queued" ? search.status : undefined,
    template: typeof search.template === "string" && search.template.length > 0 ? search.template : undefined,
  }),
  component: lazyRouteComponent(() => import("@/components/settings/mail-log-page"), "MailLogPage"),
});
