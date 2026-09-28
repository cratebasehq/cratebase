import { Database, ListTree, Mail, Plug, Shield, SlidersHorizontal, Zap, type LucideIcon } from "lucide-react";

/** One tab inside a settings group page — what used to be its own
 * top-level sidebar item before the settings navigation was consolidated
 * to {@link SETTINGS_GROUPS.length} groups. `legacyPath` is the old
 * `/settings/*` route that page lived at; the route registered there now
 * (see `routes/settings-*.tsx`) just redirects here with `?tab=value`,
 * and every page component's own `settingsItemFor(legacyPath)` call
 * keeps resolving to this tab's label/description unchanged. */
export interface SettingsTab {
  value: string;
  label: string;
  description: string;
  legacyPath: string;
  /** The system collection this tab is the front-end for, if any — lets a
   * collection page cross-link back to the settings tab that manages it. */
  collection?: string;
}

export interface SettingsGroup {
  to: string;
  label: string;
  icon: LucideIcon;
  description: string;
  tabs: SettingsTab[];
}

/**
 * The single registry every settings surface reads from — the sidebar
 * (`SettingsNav`), the breadcrumb trail, the command palette, and each
 * page's own `SettingsPage` title/description. One place to add a page or
 * rename one, instead of three lists that can (and did) disagree.
 *
 * Grouped into 7 sidebar entries (the owner's ask: keep the settings
 * sidebar lean), each a tabbed page — the tab lives in the `tab` URL
 * search param (e.g. `/settings/email?tab=templates`), not the path, so
 * it's bookmarkable and back/forward-friendly without multiplying routes.
 */
export const SETTINGS_GROUPS: SettingsGroup[] = [
  {
    to: "/settings/application",
    label: "Application",
    icon: SlidersHorizontal,
    description: "How this instance identifies itself, its branding, and which optional modules are on.",
    tabs: [
      {
        value: "general",
        label: "General",
        description: "Name, public URL, and sender identity used in emails and links.",
        legacyPath: "/settings/application",
      },
      {
        value: "branding",
        label: "Branding",
        description: "Logo and colors used in system emails and the email-template editor's default theme.",
        legacyPath: "/settings/branding",
      },
      {
        value: "modules",
        label: "Modules",
        description: "The batch API and optional built-in modules (Teams, Queue, ZIP export).",
        legacyPath: "/settings/modules",
      },
    ],
  },
  {
    to: "/settings/email",
    label: "Email",
    icon: Mail,
    description: "Delivery, templates, triggers, and the send log.",
    tabs: [
      {
        value: "delivery",
        label: "Delivery",
        description: "SMTP for outgoing mail, and S3-compatible file storage.",
        legacyPath: "/settings/mail-storage",
      },
      {
        value: "templates",
        label: "Templates",
        description: "Design and edit the emails this instance sends, and who else may send them.",
        legacyPath: "/settings/email-templates",
        collection: "_emailTemplates",
      },
      {
        value: "triggers",
        label: "Triggers",
        description: "Send an email template automatically when a record event fires.",
        legacyPath: "/settings/email-triggers",
        collection: "_emailTriggers",
      },
      {
        value: "mail-log",
        label: "Mail log",
        description: "Every send attempt through POST /api/mails/send and _emailTriggers, sent or failed.",
        legacyPath: "/settings/mail-log",
        collection: "_mailLog",
      },
      {
        value: "dev-inbox",
        label: "Dev inbox",
        description:
          "Every email the zero-config Log backend has \"sent\" — only shown while no real SMTP transport is configured.",
        legacyPath: "/settings/mail-inbox",
      },
    ],
  },
  {
    to: "/settings/auth",
    label: "Auth & security",
    icon: Shield,
    description: "Superusers, live sessions, API keys, and network protections.",
    tabs: [
      {
        value: "superusers",
        label: "Superusers",
        description: "Full-access accounts for administering this server.",
        legacyPath: "/settings/superusers",
        collection: "_superusers",
      },
      {
        value: "sessions",
        label: "Sessions",
        description: "Every live bearer/cookie token across every auth collection — revoke one, or browse who is signed in.",
        legacyPath: "/settings/sessions",
        collection: "_sessions",
      },
      {
        value: "api-keys",
        label: "API keys",
        description: "Bearer credentials for scripts, CI jobs, and MCP clients.",
        legacyPath: "/settings/api-keys",
        collection: "_api_keys",
      },
      {
        value: "network",
        label: "Network",
        description: "Rate limiting, trusted proxy, and superuser IP allowlisting.",
        legacyPath: "/settings/network",
      },
    ],
  },
  {
    to: "/settings/database",
    label: "Database",
    icon: Database,
    description: "Ad-hoc SQL, extensions, RPC functions, backups, and stored files.",
    tabs: [
      {
        value: "sql",
        label: "SQL console",
        description: "Ad-hoc SQL against the live database.",
        legacyPath: "/settings/sql",
      },
      {
        value: "extensions",
        label: "Extensions",
        description: "Enable Postgres extensions like PostGIS, pgvector, and pg_trgm. Postgres only.",
        legacyPath: "/settings/extensions",
      },
      {
        value: "rpc",
        label: "RPC functions",
        description: "Named, parameterized SQL callable as POST /api/rpc/{name}, gated by its own rule.",
        legacyPath: "/settings/rpc",
        collection: "_rpc",
      },
      {
        value: "backups",
        label: "Backups",
        description: "Full-database snapshots, stored alongside your uploaded files. SQLite only.",
        legacyPath: "/settings/backups",
      },
      {
        value: "files",
        label: "File manager",
        description: "Browse and manage files in the configured storage backend.",
        legacyPath: "/settings/file-manager",
      },
    ],
  },
  {
    to: "/settings/automation",
    label: "Automation",
    icon: Zap,
    description: "Scheduled jobs, webhooks, and custom server-side code.",
    tabs: [
      {
        value: "cron",
        label: "Cron jobs",
        description: "Scheduled work the server runs on its own, and your own scheduled SQL.",
        legacyPath: "/settings/cron",
        collection: "_cron_jobs",
      },
      {
        value: "webhooks",
        label: "Webhooks",
        description: "POST a JSON payload to a URL when a record event fires.",
        legacyPath: "/settings/webhooks",
        collection: "_webhooks",
      },
      {
        value: "functions",
        label: "Functions",
        description: "Custom server-side JavaScript hooks and the routes they register.",
        legacyPath: "/settings/functions",
      },
    ],
  },
  {
    to: "/settings/integrations",
    label: "Integrations",
    icon: Plug,
    description: "Push notifications, the LLM provider, and Model Context Protocol.",
    tabs: [
      {
        value: "push",
        label: "Push notifications",
        description: "Web, Android, and iOS push delivery for the _push_subscriptions collection.",
        legacyPath: "/settings/push",
        collection: "_push_subscriptions",
      },
      {
        value: "llm",
        label: "LLM provider",
        description: "Backs the chat endpoint and auto-embedding for vector fields.",
        legacyPath: "/settings/llm",
        collection: "_llm_usage",
      },
      {
        value: "mcp",
        label: "MCP server",
        description: "Every non-system collection exposed to Model Context Protocol clients.",
        legacyPath: "/settings/mcp",
      },
    ],
  },
  {
    to: "/settings/logs",
    label: "Logs",
    icon: ListTree,
    description: "Every API call, and the append-only audit trail.",
    tabs: [
      {
        value: "requests",
        label: "Request logs",
        description: "Every call to /api/*, as it happens.",
        legacyPath: "/settings/logs",
      },
      {
        value: "audit",
        label: "Audit log",
        description: "Append-only history of schema, settings, and superuser changes.",
        legacyPath: "/settings/audit",
        collection: "_audit_log",
      },
    ],
  },
];

/** Flattened `{ ...tab, to, group }` — the shape a page component or the
 * command palette actually wants: the tab's own label/description plus
 * where it now lives. */
export interface SettingsTabTarget extends SettingsTab {
  to: string;
  groupLabel: string;
  groupIcon: LucideIcon;
}

export const SETTINGS_TAB_TARGETS: SettingsTabTarget[] = SETTINGS_GROUPS.flatMap((group) =>
  group.tabs.map((tab) => ({ ...tab, to: group.to, groupLabel: group.label, groupIcon: group.icon })),
);

/** Resolves a page component's own settings entry by the *old* flat path
 * it always identified itself with (`settingsItemFor("/settings/superusers")`)
 * — kept working unchanged across the navigation consolidation so none of
 * the ~30 existing page components had to change their own title/description
 * lookup, only where they're mounted. */
export function settingsItemFor(legacyPath: string): SettingsTabTarget | undefined {
  return SETTINGS_TAB_TARGETS.find((tab) => tab.legacyPath === legacyPath);
}

/** The settings tab that manages a given system collection, if any — the
 * cross-link a system collection's record view offers back to its real
 * home (group path + `?tab=value`). */
export function settingsItemForCollection(name: string): SettingsTabTarget | undefined {
  return SETTINGS_TAB_TARGETS.find((tab) => tab.collection === name);
}

/** Settings-gated tab visibility: a toggle-gated built-in module's tab
 * (currently the LLM tab and the dev-inbox tab) is hidden from the
 * sidebar, command palette, and its group's own `TabsList` until its flag
 * is on, even though the tab (and its underlying route redirect) still
 * work if visited directly. */
export function isSettingsTabVisible(
  legacyPath: string,
  settings: { llm: { enabled: boolean } } | undefined,
  devMailInboxAvailable?: boolean,
): boolean {
  if (legacyPath === "/settings/llm") return Boolean(settings?.llm.enabled);
  if (legacyPath === "/settings/mail-inbox") return devMailInboxAvailable === true;
  return true;
}
