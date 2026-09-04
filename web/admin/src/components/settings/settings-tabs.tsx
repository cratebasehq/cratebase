import { Link } from "@tanstack/react-router";
import {
  Archive,
  BarChart3,
  Bot,
  Clock,
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
} from "lucide-react";

const TABS = [
  { to: "/settings/application", label: "Application", icon: SlidersHorizontal },
  { to: "/settings/mail-storage", label: "Mail & storage", icon: Mail },
  { to: "/settings/superusers", label: "Superusers", icon: ShieldUser },
  { to: "/settings/network", label: "Network", icon: Shield },
  { to: "/settings/logs", label: "Request logs", icon: ListTree },
  { to: "/settings/audit", label: "Audit log", icon: History },
  { to: "/settings/backups", label: "Backups", icon: Archive },
  { to: "/settings/cron", label: "Cron jobs", icon: Clock },
  { to: "/settings/webhooks", label: "Webhooks", icon: Webhook },
  { to: "/settings/analytics", label: "Analytics", icon: BarChart3 },
  { to: "/settings/llm", label: "LLM", icon: Sparkles },
  { to: "/settings/api-keys", label: "API keys", icon: KeyRound },
  { to: "/settings/mcp", label: "MCP", icon: Bot },
  { to: "/settings/sql", label: "SQL console", icon: Database },
  { to: "/settings/file-manager", label: "File manager", icon: FolderOpen },
] as const;

/** Sub-navigation for the settings area, mirrors the record/settings tab
 * strip on a collection page (`CollectionPage`) but as real routes instead
 * of a `?tab=` search param, since each settings section has its own data
 * fetching and deserves its own URL. */
export function SettingsTabs() {
  return (
    <nav className="flex items-center gap-1 border-b border-border px-6">
      {TABS.map(({ to, label, icon: Icon }) => (
        <Link
          key={to}
          to={to}
          className="flex items-center gap-1.5 border-b-2 border-transparent px-3 py-2.5 text-sm font-medium text-muted-foreground transition-colors hover:text-foreground data-[status=active]:border-primary data-[status=active]:text-foreground"
        >
          <Icon className="size-3.5" />
          {label}
        </Link>
      ))}
    </nav>
  );
}
