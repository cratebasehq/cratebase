import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { McpPage } from "@/components/settings/mcp-page";

export const settingsMcpRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/mcp",
  component: McpPage,
});
