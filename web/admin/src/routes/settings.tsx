import { createRoute, Outlet } from "@tanstack/react-router";
import { appRoute } from "@/routes/app";

function SettingsLayout() {
  return (
    <div className="h-full overflow-y-auto">
      <Outlet />
    </div>
  );
}

export const settingsRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/settings",
  component: SettingsLayout,
});
