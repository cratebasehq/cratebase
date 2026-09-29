import { createRouter } from "@tanstack/react-router";
import { RouteError, RouteNotFound, RoutePending } from "@/components/app-states";
import { rootRoute } from "@/routes/root";
import { loginRoute } from "@/routes/login";
import { appRoute } from "@/routes/app";
import { dashboardIndexRoute } from "@/routes/dashboard-index";
import { collectionRoute } from "@/routes/collection";
import { settingsRoute } from "@/routes/settings";
import { settingsIndexRoute } from "@/routes/settings-index";
// The 7 consolidated settings groups — each a tabbed page (`?tab=`).
import { settingsApplicationRoute } from "@/routes/settings-application";
import { settingsEmailRoute } from "@/routes/settings-email";
import { settingsAuthRoute } from "@/routes/settings-auth";
import { settingsDatabaseRoute } from "@/routes/settings-database";
import { settingsAutomationRoute } from "@/routes/settings-automation";
import { settingsIntegrationsRoute } from "@/routes/settings-integrations";
import { settingsLogsRoute } from "@/routes/settings-logs";
// The email template editor stays a standalone full-page route.
import { settingsEmailTemplateEditorRoute } from "@/routes/settings-email-template-editor";
// Old top-level routes, kept only as redirects to a group + tab above —
// see each file's own doc comment.
import { settingsMailRoute } from "@/routes/settings-mail";
import { settingsMailInboxRoute } from "@/routes/settings-mail-inbox";
import { settingsSuperusersRoute } from "@/routes/settings-superusers";
import { settingsSessionsRoute } from "@/routes/settings-sessions";
import { settingsBackupsRoute } from "@/routes/settings-backups";
import { settingsCronRoute } from "@/routes/settings-cron";
import { settingsQueueRoute } from "@/routes/settings-queue";
import { settingsNetworkRoute } from "@/routes/settings-network";
import { settingsSqlConsoleRoute } from "@/routes/settings-sql-console";
import { settingsExtensionsRoute } from "@/routes/settings-extensions";
import { settingsRpcRoute } from "@/routes/settings-rpc";
import { settingsFileManagerRoute } from "@/routes/settings-file-manager";
import { settingsWebhooksRoute } from "@/routes/settings-webhooks";
import { settingsEmailTemplatesRoute } from "@/routes/settings-email-templates";
import { settingsEmailTriggersRoute } from "@/routes/settings-email-triggers";
import { settingsMailLogRoute } from "@/routes/settings-mail-log";
import { settingsLlmRoute } from "@/routes/settings-llm";
import { settingsApiKeysRoute } from "@/routes/settings-api-keys";
import { settingsMcpRoute } from "@/routes/settings-mcp";
import { settingsAuditRoute } from "@/routes/settings-audit";
import { settingsFunctionsRoute } from "@/routes/settings-functions";
import { settingsPushRoute } from "@/routes/settings-push";

const routeTree = rootRoute.addChildren([
  loginRoute,
  appRoute.addChildren([
    dashboardIndexRoute,
    collectionRoute,
    settingsRoute.addChildren([
      settingsIndexRoute,
      settingsApplicationRoute,
      settingsEmailRoute,
      settingsAuthRoute,
      settingsDatabaseRoute,
      settingsAutomationRoute,
      settingsIntegrationsRoute,
      settingsLogsRoute,
      settingsEmailTemplateEditorRoute,
      // Redirect-only stubs for every pre-consolidation URL.
      settingsMailRoute,
      settingsMailInboxRoute,
      settingsSuperusersRoute,
      settingsSessionsRoute,
      settingsBackupsRoute,
      settingsCronRoute,
      settingsQueueRoute,
      settingsNetworkRoute,
      settingsSqlConsoleRoute,
      settingsExtensionsRoute,
      settingsRpcRoute,
      settingsFileManagerRoute,
      settingsWebhooksRoute,
      settingsEmailTemplatesRoute,
      settingsEmailTriggersRoute,
      settingsMailLogRoute,
      settingsLlmRoute,
      settingsApiKeysRoute,
      settingsMcpRoute,
      settingsAuditRoute,
      settingsFunctionsRoute,
      settingsPushRoute,
    ]),
  ]),
]);

export const router = createRouter({
  routeTree,
  // The server mounts the dashboard at `/_/` (PocketBase's path), so every
  // route lives under that prefix. Without this the bundle loads and then
  // renders "No such page", because the router compares `/_/` against a
  // tree rooted at `/`. Kept in step with `base` in vite.config.ts.
  basepath: "/_/",
  defaultPreload: "intent",
  // Every route now has a designed failure, loading and 404 state. Before
  // this, a throw in a loader rendered nothing and an unknown URL rendered
  // the router's built-in placeholder.
  defaultErrorComponent: RouteError,
  defaultPendingComponent: RoutePending,
  defaultNotFoundComponent: RouteNotFound,
  // Long enough that a fast local backend never flashes a skeleton.
  defaultPendingMs: 250,
  defaultPendingMinMs: 320,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
