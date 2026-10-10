import type { ReactNode } from "react";
import { CircleAlert, Inbox, RotateCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

/**
 * "Nothing here" — always with the one action that fixes it. `compact` for
 * inside a card or table body, default for a whole screen. Say what the
 * thing is and what creating one gives you; never just "No data".
 */
export function EmptyState({
  icon,
  title,
  description,
  action,
  compact = false,
  className,
}: {
  icon?: ReactNode;
  title: string;
  description?: ReactNode;
  action?: ReactNode;
  compact?: boolean;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "animate-rise flex flex-col items-center justify-center gap-1 rounded-lg border border-dashed border-border text-center",
        compact ? "px-4 py-8" : "flex-1 px-6 py-16",
        className,
      )}
    >
      <span
        aria-hidden
        className="mb-2 flex size-9 items-center justify-center rounded-xl border border-border bg-card text-muted-foreground shadow-xs [&_svg]:size-4"
      >
        {icon ?? <Inbox />}
      </span>
      <p className="text-sm font-medium">{title}</p>
      {description ? <p className="max-w-measure text-sm text-pretty text-muted-foreground">{description}</p> : null}
      {action ? <div className="mt-3 flex items-center gap-2">{action}</div> : null}
    </div>
  );
}

/**
 * A failed load, in place. Shows the server's message when there is one and
 * a retry. For a whole-route failure the router already renders `RouteError`;
 * use this for a panel that failed while the rest of the page is fine.
 */
export function ErrorState({
  title = "Couldn't load this",
  error,
  onRetry,
  compact = false,
  className,
}: {
  title?: string;
  error?: unknown;
  onRetry?: () => void;
  compact?: boolean;
  className?: string;
}) {
  const message = error instanceof Error ? error.message : typeof error === "string" ? error : undefined;
  return (
    <div
      role="alert"
      className={cn(
        "flex flex-col items-center justify-center gap-1 rounded-lg border border-destructive/25 bg-destructive/5 text-center",
        compact ? "px-4 py-6" : "flex-1 px-6 py-14",
        className,
      )}
    >
      <CircleAlert aria-hidden className="mb-1 size-5 text-destructive" />
      <p className="text-sm font-medium">{title}</p>
      {message ? <p className="max-w-measure text-sm text-pretty text-muted-foreground">{message}</p> : null}
      {onRetry ? (
        <Button variant="outline" size="sm" className="mt-3" onClick={onRetry}>
          <RotateCw />
          Try again
        </Button>
      ) : null}
    </div>
  );
}

/** Rows of skeleton for a list or table body while it loads. Match `rows` to
 * what a typical page will show so the layout does not jump on arrival. */
export function ListSkeleton({ rows = 5, className }: { rows?: number; className?: string }) {
  return (
    <div className={cn("flex flex-col gap-1", className)} aria-busy="true" aria-label="Loading">
      {Array.from({ length: rows }).map((_, i) => (
        <Skeleton key={i} className="h-row w-full" style={{ opacity: 1 - i * 0.12 }} />
      ))}
    </div>
  );
}
