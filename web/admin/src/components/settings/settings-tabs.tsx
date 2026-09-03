import { Link } from "@tanstack/react-router";
import { Archive, Clock, ListTree } from "lucide-react";

const TABS = [
  { to: "/settings/logs", label: "Request logs", icon: ListTree },
  { to: "/settings/backups", label: "Backups", icon: Archive },
  { to: "/settings/cron", label: "Cron jobs", icon: Clock },
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
          className="flex items-center gap-1.5 border-b-2 border-transparent px-3 py-2.5 text-[13px] font-medium text-muted-foreground transition-colors hover:text-foreground"
          activeProps={{ className: "!border-primary !text-foreground" }}
        >
          <Icon className="size-3.5" />
          {label}
        </Link>
      ))}
    </nav>
  );
}
