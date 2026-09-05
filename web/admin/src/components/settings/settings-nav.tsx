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
 * route is under `/settings` — the six grouped sections from
 * `SETTINGS_GROUPS`, rendered with the same primitives `CollectionItem`
 * already uses in `app-sidebar.tsx`, so icon-rail collapse and tooltips
 * come free instead of needing a second nav implementation.
 */
export function SettingsNav({ pathname }: { pathname: string }) {
  return (
    <>
      {SETTINGS_GROUPS.map((group) => (
        <SidebarGroup key={group.label}>
          <SidebarGroupLabel>{group.label}</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {group.items.map(({ to, label, icon: Icon }) => (
                <SidebarMenuItem key={to}>
                  <SidebarMenuButton asChild isActive={pathname === to} tooltip={label}>
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
      ))}
    </>
  );
}
