import { createRoute, lazyRouteComponent } from "@tanstack/react-router";
import { settingsRoute } from "@/routes/settings";

/** `$id` is either a real `_emailTemplates` record id, or the literal
 * `"new"` — the editor page itself tells the two apart. A dedicated,
 * lazy-loaded route (rather than a dialog, like `_webhooks`/`_cron_jobs`)
 * because the editor (CodeMirror/visual editor, live preview, test send)
 * is heavy enough to want its own code-split chunk, matching how
 * `settings-sql-console`/`settings-webhooks` already split their own
 * pages. */
export const settingsEmailTemplateEditorRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/email-templates/$id",
  component: lazyRouteComponent(
    () => import("@/components/settings/email-template-editor-page"),
    "EmailTemplateEditorPage",
  ),
});
