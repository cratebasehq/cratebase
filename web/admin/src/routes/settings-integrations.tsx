import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

export type IntegrationsTab = "push" | "llm" | "mcp";

export type IntegrationsSearch = { tab?: IntegrationsTab };

function isIntegrationsTab(value: unknown): value is IntegrationsTab {
  return value === "push" || value === "llm" || value === "mcp";
}

/** `/settings/integrations` — the "Integrations" settings group: Push
 * notifications, LLM provider, and MCP server as tabs. */
export const settingsIntegrationsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/integrations",
  validateSearch: (search: Record<string, unknown>): IntegrationsSearch => ({
    tab: isIntegrationsTab(search.tab) ? search.tab : undefined,
  }),
  component: lazyRouteComponent(() => import("@/components/settings/integrations-settings-page"), "IntegrationsSettingsPage"),
});
