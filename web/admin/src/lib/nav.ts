import {
  Activity,
  Bot,
  Brain,
  Clock,
  Cpu,
  FileBox,
  HardDrive,
  KeyRound,
  LayoutGrid,
  ListTree,
  Mail,
  Network,
  Plug,
  Radio,
  ScrollText,
  Shield,
  SlidersHorizontal,
  Terminal,
  Users,
  Webhook,
  type LucideIcon,
} from "lucide-react";
import { SETTINGS_GROUPS } from "@/lib/settings-nav";

/**
 * The sidebar and the command palette read the same registry, so a screen is
 * either reachable from both or from neither. Settings screens are tabs of
 * the seven `SETTINGS_GROUPS` pages; here they are re-grouped by what a
 * developer is *doing* (Data / Auth / Automation / AI / System) instead of by
 * where the settings code happens to live.
 *
 * `shortcut` is a two-key chord (`g` then the letter), shown as a hint in the
 * sidebar and the palette and handled by `useNavShortcuts`.
 */
export interface NavItem {
  id: string;
  label: string;
  to: string;
  /** Settings tab value; omitted for the group's default tab / non-tab routes. */
  tab?: string;
  icon: LucideIcon;
  /** Second key of the `g` chord. */
  chord?: string;
  /** Keywords for the palette beyond the label. */
  keywords?: string;
  /** `/settings/llm` & co: the legacy path `isSettingsTabVisible` knows. */
  legacyPath?: string;
}

export interface NavGroup {
  id: "data" | "auth" | "automation" | "ai" | "system";
  label: string;
  items: NavItem[];
}

export const OVERVIEW: NavItem = { id: "overview", label: "Overview", to: "/", icon: LayoutGrid, chord: "o" };

export const NAV_GROUPS: NavGroup[] = [
  {
    id: "data",
    label: "Data",
    // Collections are dynamic and rendered by the sidebar itself; these are
    // the static tools that work on data.
    items: [
      { id: "sql", label: "SQL console", to: "/settings/database", tab: "sql", icon: Terminal, chord: "q", keywords: "query database" },
      { id: "files", label: "File manager", to: "/settings/database", tab: "files", icon: FileBox, keywords: "storage uploads" },
      { id: "backups", label: "Backups", to: "/settings/database", tab: "backups", icon: HardDrive, keywords: "snapshot restore" },
    ],
  },
  {
    id: "auth",
    label: "Auth",
    items: [
      { id: "superusers", label: "Superusers", to: "/settings/auth", tab: "superusers", icon: Shield, keywords: "admins" },
      { id: "sessions", label: "Sessions", to: "/settings/auth", tab: "sessions", icon: Users, keywords: "tokens signed in" },
      { id: "api-keys", label: "API keys", to: "/settings/auth", tab: "api-keys", icon: KeyRound, chord: "k", keywords: "tokens bearer" },
      { id: "network", label: "Network", to: "/settings/auth", tab: "network", icon: Network, keywords: "rate limit proxy ip" },
    ],
  },
  {
    id: "automation",
    label: "Automation",
    items: [
      { id: "cron", label: "Cron jobs", to: "/settings/automation", tab: "cron", icon: Clock, keywords: "schedule" },
      { id: "webhooks", label: "Webhooks", to: "/settings/automation", tab: "webhooks", icon: Webhook },
      { id: "queue", label: "Queue", to: "/settings/automation", tab: "queue", icon: Activity, chord: "j", keywords: "jobs background" },
      { id: "functions", label: "Functions", to: "/settings/automation", tab: "functions", icon: Cpu, keywords: "hooks javascript" },
      { id: "realtime", label: "Realtime channels", to: "/settings/automation", tab: "realtime", icon: Radio, keywords: "websocket sse" },
    ],
  },
  {
    id: "ai",
    label: "AI",
    items: [
      { id: "llm", label: "LLM gateway", to: "/settings/integrations", tab: "llm", icon: Brain, legacyPath: "/settings/llm", keywords: "model provider embeddings usage" },
      { id: "mcp", label: "MCP server", to: "/settings/integrations", tab: "mcp", icon: Bot, keywords: "model context protocol" },
    ],
  },
  {
    id: "system",
    label: "System",
    items: [
      { id: "application", label: "Application", to: "/settings/application", icon: SlidersHorizontal, chord: "s", keywords: "general branding modules storage" },
      { id: "email", label: "Email", to: "/settings/email", icon: Mail, keywords: "smtp templates" },
      { id: "integrations", label: "Integrations", to: "/settings/integrations", tab: "push", icon: Plug, keywords: "push notifications" },
      { id: "logs", label: "Logs", to: "/settings/logs", icon: ListTree, chord: "l", keywords: "requests audit" },
      { id: "audit", label: "Audit log", to: "/settings/logs", tab: "audit", icon: ScrollText },
    ],
  },
];

/** Every destination, flat — the palette's "Go to" list. */
export const ALL_NAV_ITEMS: NavItem[] = [OVERVIEW, ...NAV_GROUPS.flatMap((g) => g.items)];

/** The tab a group page opens on when the URL has none. */
function defaultTab(to: string): string | undefined {
  return SETTINGS_GROUPS.find((g) => g.to === to)?.tabs[0]?.value;
}

export function isNavItemActive(item: NavItem, pathname: string, tab: string | undefined): boolean {
  if (item.to === "/") return pathname === "/";
  if (pathname !== item.to && !pathname.startsWith(`${item.to}/`)) return false;
  // Items that share a page (all of Automation) are told apart by tab.
  const sharesPage = ALL_NAV_ITEMS.filter((i) => i.to === item.to).length > 1;
  if (!sharesPage) return true;
  return (tab ?? defaultTab(item.to)) === (item.tab ?? defaultTab(item.to));
}

export function navLinkProps(item: NavItem) {
  return { to: item.to, search: item.tab ? { tab: item.tab } : undefined } as never;
}
