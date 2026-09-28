import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type LogsTab = "requests" | "audit";

function isLogsTab(value: unknown): value is LogsTab {
  return value === "requests" || value === "audit";
}

/** The union of Request logs' and Audit log's own search params, merged
 * flat under one route. Both tabs use `page` for their own pagination —
 * since only one is ever showing, they share the key (switching tabs
 * resets it, which `AuditLogPage`/`RequestLogsPage`'s own tab-change
 * handling clears anyway). `filter` is Request logs' only; `action`/
 * `from`/`to` are Audit log's only. */
export type LogsSearch = {
  tab?: LogsTab;
  page?: number;
  filter?: string;
  action?: string;
  from?: string;
  to?: string;
};

/** `/settings/logs` — Request logs and Audit log as tabs. */
export const settingsLogsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/logs",
  validateSearch: (search: Record<string, unknown>): LogsSearch => ({
    tab: isLogsTab(search.tab) ? search.tab : undefined,
    page: typeof search.page === "number" && search.page > 1 ? search.page : undefined,
    filter: typeof search.filter === "string" && search.filter.length > 0 ? search.filter : undefined,
    action: typeof search.action === "string" && search.action.length > 0 ? search.action : undefined,
    from: typeof search.from === "string" && search.from.length > 0 ? search.from : undefined,
    to: typeof search.to === "string" && search.to.length > 0 ? search.to : undefined,
  }),
  // Lazy: this tab pulls in recharts for the request-volume chart.
  component: lazyRouteComponent(() => import("@/components/settings/logs-settings-page"), "LogsSettingsPage"),
});
