import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type EmailTab = "delivery" | "templates" | "triggers" | "mail-log" | "dev-inbox";

/** The union of every tab's own search params, merged flat — only one tab
 * is ever visible at a time, and none of their key names collide (`page`
 * is unique to Mail log here; Request/Audit logs under the Logs group are
 * the ones that share `page`). */
export type EmailSearch = {
  tab?: EmailTab;
  /** Mail log */
  page?: number;
  status?: "sent" | "failed" | "queued";
  template?: string;
  /** Dev inbox */
  id?: string;
};

function isEmailTab(value: unknown): value is EmailTab {
  return value === "delivery" || value === "templates" || value === "triggers" || value === "mail-log" || value === "dev-inbox";
}

/** `/settings/email` — Delivery, Templates, Triggers, Mail log, and (while
 * the zero-config Log backend is active) Dev inbox, as tabs. The
 * template *editor* stays a separate full-page route
 * (`settings-email-template-editor.tsx`) reachable from the Templates
 * tab — heavy enough (CodeMirror/visual editor/live preview) to want its
 * own code-split chunk and its own URL, not a tab. */
export const settingsEmailRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/email",
  validateSearch: (search: Record<string, unknown>): EmailSearch => ({
    tab: isEmailTab(search.tab) ? search.tab : undefined,
    page: typeof search.page === "number" && search.page > 1 ? search.page : undefined,
    status:
      search.status === "sent" || search.status === "failed" || search.status === "queued" ? search.status : undefined,
    template: typeof search.template === "string" && search.template.length > 0 ? search.template : undefined,
    id: typeof search.id === "string" && search.id.length > 0 ? search.id : undefined,
  }),
  component: lazyRouteComponent(() => import("@/components/settings/email-settings-page"), "EmailSettingsPage"),
});
