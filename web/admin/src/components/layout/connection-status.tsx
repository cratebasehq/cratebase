import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { checkHealth } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

export type ConnectionState = "connecting" | "connected" | "degraded" | "offline";

const POLL_MS = 20_000;

/**
 * Tracks whether the dashboard can still talk to its server.
 *
 * Deliberately reports what it actually measures: the browser's own
 * online/offline signal, plus a periodic unauthenticated `/api/health` call
 * with its round-trip time. "Degraded" means the browser thinks it is online
 * but the server stopped answering — the state where a self-hosted backend
 * has usually just been restarted.
 */
export function useConnectionStatus(): { state: ConnectionState; latencyMs: number | null } {
  const queryClient = useQueryClient();
  const [online, setOnline] = useState(() => typeof navigator === "undefined" || navigator.onLine);

  // One shared query, not one poller per caller: the topbar and the
  // dashboard both want this, and two independent probes would race (and,
  // sharing a request key, cancel each other).
  const probe = useQuery({
    queryKey: ["health"],
    queryFn: async () => {
      const startedAt = performance.now();
      await checkHealth();
      return Math.round(performance.now() - startedAt);
    },
    refetchInterval: POLL_MS,
    refetchIntervalInBackground: false,
    retry: false,
    gcTime: POLL_MS * 3,
    enabled: online,
  });

  useEffect(() => {
    const onOnline = () => {
      setOnline(true);
      void queryClient.invalidateQueries({ queryKey: ["health"] });
    };
    const onOffline = () => setOnline(false);
    window.addEventListener("online", onOnline);
    window.addEventListener("offline", onOffline);
    return () => {
      window.removeEventListener("online", onOnline);
      window.removeEventListener("offline", onOffline);
    };
  }, [queryClient]);

  const state: ConnectionState = !online
    ? "offline"
    : probe.isSuccess
      ? "connected"
      : probe.isError
        ? "degraded"
        : "connecting";

  return { state, latencyMs: probe.data ?? null };
}

const LABEL: Record<ConnectionState, string> = {
  connecting: "Connecting",
  connected: "Connected",
  degraded: "No response",
  offline: "Offline",
};

const DETAIL: Record<ConnectionState, string> = {
  connecting: "Checking the connection to your Cratebase server.",
  connected: "The server is answering.",
  degraded: "The browser is online but the server stopped answering. It may be restarting.",
  offline: "This browser has no network connection.",
};

export function ConnectionStatus({ className }: { className?: string }) {
  const { state, latencyMs } = useConnectionStatus();

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          className={cn(
            "flex h-control-sm shrink-0 items-center gap-1.5 rounded-md px-1.5 text-xs whitespace-nowrap text-muted-foreground",
            className,
          )}
        >
          <span
            aria-hidden
            className={cn(
              "size-1.5 shrink-0 rounded-full",
              state === "connecting" && "bg-muted-foreground motion-safe:animate-pulse",
              state === "connected" && "bg-success",
              state === "degraded" && "bg-warning motion-safe:animate-pulse",
              state === "offline" && "bg-destructive",
            )}
          />
          <span className="sr-only sm:not-sr-only">{LABEL[state]}</span>
        </span>
      </TooltipTrigger>
      <TooltipContent side="bottom">
        <p>{DETAIL[state]}</p>
        {latencyMs !== null ? (
          <p className="font-tabular opacity-70">GET /api/health · {latencyMs} ms</p>
        ) : null}
      </TooltipContent>
    </Tooltip>
  );
}
