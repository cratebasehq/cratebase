import { RefreshCw } from "lucide-react";
import { StatusDot } from "@/components/layout/stat-card";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** Polling interval for live views. react-query already pauses interval
 * refetching while the tab is hidden (`refetchIntervalInBackground` is off). */
export const LIVE_INTERVAL_MS = 10_000;

/**
 * Small "Live" switch + manual refresh for a polling view. `live` is a
 * pressed toggle (the pulse only runs while it is on); the refresh icon spins
 * while a fetch is in flight. Pair with
 * `refetchInterval: live && !paused ? LIVE_INTERVAL_MS : false`.
 */
export function LiveToggle({
  live,
  onLiveChange,
  fetching,
  onRefresh,
}: {
  live: boolean;
  onLiveChange: (live: boolean) => void;
  fetching: boolean;
  onRefresh: () => void;
}) {
  return (
    <div className="flex items-center gap-1">
      <Button
        type="button"
        variant="outline"
        size="sm"
        aria-pressed={live}
        title={live ? "Auto-refreshing every 10 s — click to pause" : "Auto-refresh paused — click to resume"}
        onClick={() => onLiveChange(!live)}
      >
        <StatusDot tone={live ? "success" : "neutral"} pulse={live} />
        {live ? "Live" : "Paused"}
      </Button>
      <Button type="button" variant="ghost" size="icon-sm" aria-label="Refresh now" title="Refresh now" onClick={onRefresh}>
        <RefreshCw className={cn(fetching && "animate-spin")} />
      </Button>
    </div>
  );
}
