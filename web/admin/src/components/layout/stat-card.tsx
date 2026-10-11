import type { ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { cn } from "@/lib/utils";
import { Skeleton } from "@/components/ui/skeleton";

export type Tone = "neutral" | "success" | "warning" | "danger" | "info" | "brand";

const DOT: Record<Tone, string> = {
  neutral: "bg-muted-foreground/60",
  success: "bg-success",
  warning: "bg-warning",
  danger: "bg-destructive",
  info: "bg-info",
  brand: "bg-primary",
};

const SOFT: Record<Tone, string> = {
  neutral: "bg-muted text-muted-foreground",
  success: "bg-success/12 text-success",
  warning: "bg-warning/14 text-warning",
  danger: "bg-destructive/12 text-destructive",
  info: "bg-info/12 text-info",
  brand: "bg-primary/14 text-brand-text",
};

/** A coloured dot. `pulse` adds the one ambient "live" ring; pass it only for
 * state that is actively changing (connecting, running), never for decoration. */
export function StatusDot({ tone = "neutral", pulse = false, className }: { tone?: Tone; pulse?: boolean; className?: string }) {
  return (
    <span aria-hidden className={cn("relative inline-flex size-1.5 shrink-0", className)}>
      {pulse ? (
        <span className={cn("absolute inset-0 rounded-full motion-safe:animate-[status-ping_1.6s_var(--ease-out-strong)_infinite]", DOT[tone])} />
      ) : null}
      <span className={cn("relative size-1.5 rounded-full", DOT[tone])} />
    </span>
  );
}

/** Semantic status chip: dot + label on a tinted wash. Colour is never the
 * only signal — the label always says what the state is. */
export function StatusBadge({
  tone = "neutral",
  children,
  className,
}: {
  tone?: Tone;
  children: ReactNode;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex h-5 items-center gap-1.5 rounded-full px-2 text-xs font-medium whitespace-nowrap",
        SOFT[tone],
        className,
      )}
    >
      <StatusDot tone={tone} />
      {children}
    </span>
  );
}

/**
 * One headline number with a label and an optional hint, for a row of
 * at-a-glance facts. Pass `to` to make the whole card a link to the screen
 * that owns the number. `loading` shows a skeleton where the value will be so
 * the row never jumps; `tone` colours the value only when it is non-zero-ish
 * bad news (`danger`) or good news (`success`) — leave it `neutral` otherwise.
 */
export function StatCard({
  label,
  value,
  hint,
  icon,
  tone = "neutral",
  loading = false,
  to,
  search,
  index = 0,
  className,
}: {
  label: string;
  value: ReactNode;
  hint?: ReactNode;
  icon?: ReactNode;
  tone?: Tone;
  loading?: boolean;
  to?: string;
  /** Search params for `to` (e.g. `{ tab: "queue" }`). */
  search?: Record<string, string>;
  /** Position in a row, for the entrance stagger. */
  index?: number;
  className?: string;
}) {
  const body = (
    <>
      <div className="flex items-center justify-between gap-2 text-muted-foreground">
        <span className="truncate text-xs">{label}</span>
        {icon ? <span className="shrink-0 [&_svg]:size-3.5">{icon}</span> : null}
      </div>
      {loading ? (
        <Skeleton className="mt-1 h-7 w-16" />
      ) : (
        <span
          className={cn(
            "font-tabular text-2xl leading-8 font-semibold tracking-tight",
            tone === "danger" && "text-destructive",
            tone === "success" && "text-success",
            tone === "warning" && "text-warning",
          )}
        >
          {value}
        </span>
      )}
      {hint ? <span className="truncate text-xs text-muted-foreground">{hint}</span> : null}
    </>
  );
  const classes = cn(
    "animate-rise flex min-w-0 flex-col gap-0.5 rounded-lg border border-border bg-card px-3 py-2.5 shadow-xs",
    to && "transition-[border-color,box-shadow] hover:border-border-strong/60 hover:shadow-e2",
    className,
  );
  const style = { "--i": index } as React.CSSProperties;
  return to ? (
    <Link to={to as never} search={search as never} className={classes} style={style}>
      {body}
    </Link>
  ) : (
    <div className={classes} style={style}>
      {body}
    </div>
  );
}
