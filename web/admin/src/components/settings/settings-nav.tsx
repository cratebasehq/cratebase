import { Link } from "@tanstack/react-router";
import { SETTINGS_GROUPS } from "@/lib/settings-nav";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";

/**
 * Replaces the Collections/System groups in the app sidebar while the
 * route is under `/settings` — one entry per `SETTINGS_GROUPS` group (≤7,
 * the owner's ask for a lean settings sidebar), each a tabbed page rather
 * than its own route. Every group always has at least one always-visible
 * tab, so unlike the old per-page list this never needs to hide a whole
 * row — a toggle-gated tab (LLM, dev inbox) just doesn't show up inside
 * its group's own `TabsList` until enabled.
 */
export function SettingsNav({ pathname }: { pathname: string }) {
  return (
    <SidebarGroup>
      <SidebarGroupLabel>Settings</SidebarGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {SETTINGS_GROUPS.map(({ to, label, icon: Icon }) => (
            <SidebarMenuItem key={to}>
              <SidebarMenuButton asChild isActive={pathname === to || pathname.startsWith(`${to}/`)} tooltip={label}>
                <Link to={to}>
                  <Icon />
                  <span>{label}</span>
                </Link>
              </SidebarMenuButton>
            </SidebarMenuItem>
          ))}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}
