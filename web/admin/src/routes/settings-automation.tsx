import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type AutomationTab = "cron" | "webhooks" | "queue" | "functions" | "realtime";

export type AutomationSearch = { tab?: AutomationTab };

function isAutomationTab(value: unknown): value is AutomationTab {
  return (
    value === "cron" ||
    value === "webhooks" ||
    value === "queue" ||
    value === "functions" ||
    value === "realtime"
  );
}

/** `/settings/automation` — the "Automation" settings group: Cron jobs,
 * Webhooks, Functions, and Realtime channels as tabs. */
export const settingsAutomationRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/automation",
  validateSearch: (search: Record<string, unknown>): AutomationSearch => ({
    tab: isAutomationTab(search.tab) ? search.tab : undefined,
  }),
  component: lazyRouteComponent(() => import("@/components/settings/automation-settings-page"), "AutomationSettingsPage"),
});
