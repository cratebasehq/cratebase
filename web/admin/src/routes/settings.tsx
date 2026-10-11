import { createRoute, Outlet } from "@tanstack/react-router";
import { appRoute } from "@/routes/app";

function SettingsLayout() {
  return (
    <div className="flex h-full min-h-0 flex-col overflow-y-auto">
      <Outlet />
    </div>
  );
}

export const settingsRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/settings",
  component: SettingsLayout,
});
