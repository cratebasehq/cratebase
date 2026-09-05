import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type AuditLogSearch = {
  page?: number;
  action?: string;
  from?: string;
  to?: string;
};

export const settingsAuditRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/audit",
  validateSearch: (search: Record<string, unknown>): AuditLogSearch => ({
    page: typeof search.page === "number" && search.page > 1 ? search.page : undefined,
    action: typeof search.action === "string" && search.action.length > 0 ? search.action : undefined,
    from: typeof search.from === "string" && search.from.length > 0 ? search.from : undefined,
    to: typeof search.to === "string" && search.to.length > 0 ? search.to : undefined,
  }),
  component: lazyRouteComponent(() => import("@/components/settings/audit-log-page"), "AuditLogPage"),
});
