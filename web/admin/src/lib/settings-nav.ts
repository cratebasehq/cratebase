import {
  Archive,
  Bell,
  Bot,
  Clock,
  Code2,
  Database,
  FolderOpen,
  History,
  KeyRound,
  ListTree,
  Mail,
  Shield,
  ShieldUser,
  SlidersHorizontal,
  Sparkles,
  Webhook,
  type LucideIcon,
} from "lucide-react";

export interface SettingsItem {
  to: string;
  label: string;
  icon: LucideIcon;
  description: string;
  /** The system collection this page is the front-end for, if any — lets a
   * collection page cross-link back to the settings page that manages it. */
  collection?: string;
}

export interface SettingsGroup {
  label: string;
  items: SettingsItem[];
}

/**
 * The single registry every settings surface reads from — the sidebar
 * (`SettingsNav`), the breadcrumb trail, the command palette, and each
 * page's own `SettingsPage` title/description. One place to add a page or
 * rename one, instead of three lists that can (and did) disagree.
 */
export const SETTINGS_GROUPS: SettingsGroup[] = [
  {
    label: "General",
    items: [
      {
        to: "/settings/application",
        label: "Application",
        icon: SlidersHorizontal,
        description: "How this instance identifies itself in emails and links, plus batch API and request log settings.",
      },
      {
        to: "/settings/mail-storage",
        label: "Mail & storage",
        icon: Mail,
        description: "SMTP for outgoing mail and S3-compatible file storage.",
      },
      {
        to: "/settings/network",
        label: "Network",
        icon: Shield,
        description: "Rate limiting, trusted proxy, and superuser IP allowlisting.",
      },
    ],
  },
  {
    label: "Access",
    items: [
      {
        to: "/settings/superusers",
        label: "Superusers",
        icon: ShieldUser,
        description: "Full-access accounts for administering this server.",
        collection: "_superusers",
      },
      {
        to: "/settings/api-keys",
        label: "API keys",
        icon: KeyRound,
        description: "Bearer credentials for scripts, CI jobs, and MCP clients.",
        collection: "_api_keys",
      },
    ],
  },
  {
    label: "Data",
    items: [
      {
        to: "/settings/backups",
        label: "Backups",
        icon: Archive,
        description: "Full-database snapshots, stored alongside your uploaded files. SQLite only.",
      },
      {
        to: "/settings/sql",
        label: "SQL console",
        icon: Database,
        description: "Ad-hoc SQL against the live database.",
      },
      {
        to: "/settings/file-manager",
        label: "File manager",
        icon: FolderOpen,
        description: "Browse and manage files in the configured storage backend.",
      },
    ],
  },
  {
    label: "Automation",
    items: [
      {
        to: "/settings/cron",
        label: "Cron jobs",
        icon: Clock,
        description: "Scheduled work the server runs on its own, and your own scheduled SQL.",
        collection: "_cron_jobs",
      },
      {
        to: "/settings/webhooks",
        label: "Webhooks",
        icon: Webhook,
        description: "POST a JSON payload to a URL when a record event fires.",
        collection: "_webhooks",
      },
      {
        to: "/settings/push",
        label: "Push notifications",
        icon: Bell,
        description: "Web, Android, and iOS push delivery for the _push_subscriptions collection.",
        collection: "_push_subscriptions",
      },
      {
        to: "/settings/functions",
        label: "Functions",
        icon: Code2,
        description: "Custom server-side JavaScript hooks and the routes they register.",
      },
    ],
  },
  {
    label: "AI",
    items: [
      {
        to: "/settings/llm",
        label: "LLM provider",
        icon: Sparkles,
        description: "Backs the chat endpoint and auto-embedding for vector fields.",
        collection: "_llm_usage",
      },
      {
        to: "/settings/mcp",
        label: "MCP server",
        icon: Bot,
        description: "Every non-system collection exposed to Model Context Protocol clients.",
      },
    ],
  },
  {
    label: "Observe",
    items: [
      {
        to: "/settings/logs",
        label: "Request logs",
        icon: ListTree,
        description: "Every call to /api/*, as it happens.",
      },
      {
        to: "/settings/audit",
        label: "Audit log",
        icon: History,
        description: "Append-only history of schema, settings, and superuser changes.",
        collection: "_audit_log",
      },
    ],
  },
];

export const SETTINGS_ITEMS: SettingsItem[] = SETTINGS_GROUPS.flatMap((group) => group.items);

/** The item whose route the given pathname is under — `/settings/logs/x`
 * still resolves to the `Request logs` item, matching how the router
 * nests search-param state under these fixed paths. */
export function settingsItemFor(pathname: string): SettingsItem | undefined {
  return SETTINGS_ITEMS.find((item) => pathname === item.to || pathname.startsWith(`${item.to}/`));
}

/** The settings item that manages a given system collection, if any — the
 * cross-link a system collection's record view offers back to its real
 * home. */
export function settingsItemForCollection(name: string): SettingsItem | undefined {
  return SETTINGS_ITEMS.find((item) => item.collection === name);
}
