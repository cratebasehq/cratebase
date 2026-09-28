import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type AutomationTab = "cron" | "webhooks" | "functions";

export type AutomationSearch = { tab?: AutomationTab };

function isAutomationTab(value: unknown): value is AutomationTab {
  return value === "cron" || value === "webhooks" || value === "functions";
}

/** `/settings/automation` — the "Automation" settings group: Cron jobs,
 * Webhooks, and Functions as tabs. */
export const settingsAutomationRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/automation",
  validateSearch: (search: Record<string, unknown>): AutomationSearch => ({
    tab: isAutomationTab(search.tab) ? search.tab : undefined,
  }),
  component: lazyRouteComponent(() => import("@/components/settings/automation-settings-page"), "AutomationSettingsPage"),
});
