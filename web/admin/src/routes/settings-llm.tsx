import { createRoute } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";
import { LlmPage } from "@/components/settings/llm-page";

export const settingsLlmRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/llm",
  component: LlmPage,
});
