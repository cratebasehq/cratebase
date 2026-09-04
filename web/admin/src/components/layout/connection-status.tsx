import { useEffect, useState } from "react";
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
  const [state, setState] = useState<ConnectionState>("connecting");
  const [latencyMs, setLatencyMs] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;

    async function ping() {
      if (cancelled) return;
      if (typeof navigator !== "undefined" && !navigator.onLine) {
        setState("offline");
        setLatencyMs(null);
      } else {
        const startedAt = performance.now();
        try {
          await checkHealth();
          if (cancelled) return;
          setLatencyMs(Math.round(performance.now() - startedAt));
          setState("connected");
        } catch {
          if (cancelled) return;
          setLatencyMs(null);
          setState(typeof navigator !== "undefined" && !navigator.onLine ? "offline" : "degraded");
        }
      }
      timer = setTimeout(ping, POLL_MS);
    }

    void ping();

    const onOnline = () => void ping();
    const onOffline = () => setState("offline");
    window.addEventListener("online", onOnline);
    window.addEventListener("offline", onOffline);

    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
      window.removeEventListener("online", onOnline);
      window.removeEventListener("offline", onOffline);
    };
  }, []);

  return { state, latencyMs };
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
            "flex h-control-sm items-center gap-1.5 rounded-md px-1.5 text-xs text-muted-foreground",
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
