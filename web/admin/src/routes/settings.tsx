import { createRoute, Outlet } from "@tanstack/react-router";
import { appRoute } from "@/routes/app";
import { SettingsTabs } from "@/components/settings/settings-tabs";

function SettingsLayout() {
  return (
    <div className="flex h-full flex-col">
      <div className="border-b border-border px-6 py-4">
        <h1 className="text-sm font-semibold">Settings</h1>
        <p className="mt-0.5 text-[12.5px] text-muted-foreground">
          Server-level operations: request history, database backups, and scheduled jobs.
        </p>
      </div>
      <SettingsTabs />
      <div className="flex-1 overflow-y-auto">
        <Outlet />
      </div>
    </div>
  );
}

export const settingsRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/settings",
  component: SettingsLayout,
});
