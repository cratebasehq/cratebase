import { parseServerDate } from "@/lib/api";
import { cn } from "@/lib/utils";

const AGO = new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style: "short" });

/** "3 min. ago" / "in 2 days" for a timestamp. Callers re-render with their
 * query, so no ticking clock is needed. */
export function relativeTime(value: string | Date): string {
  const date = typeof value === "string" ? parseServerDate(value) : value;
  const seconds = Math.round((date.getTime() - Date.now()) / 1000);
  const abs = Math.abs(seconds);
  if (abs < 45) return AGO.format(seconds, "second");
  if (abs < 3600) return AGO.format(Math.round(seconds / 60), "minute");
  if (abs < 86400) return AGO.format(Math.round(seconds / 3600), "hour");
  if (abs < 86400 * 60) return AGO.format(Math.round(seconds / 86400), "day");
  return AGO.format(Math.round(seconds / (86400 * 30)), "month");
}

/** A server timestamp shown relative, with the absolute local time on hover
 * and in a machine-readable `<time>`. Renders `fallback` for empty values. */
export function TimeAgo({
  value,
  fallback = "—",
  className,
}: {
  value: string | null | undefined;
  fallback?: string;
  className?: string;
}) {
  if (!value) return <span className={cn("text-muted-foreground", className)}>{fallback}</span>;
  const date = parseServerDate(value);
  if (Number.isNaN(date.getTime())) return <span className={className}>{value}</span>;
  return (
    <time dateTime={date.toISOString()} title={date.toLocaleString()} className={cn("font-tabular", className)}>
      {relativeTime(date)}
    </time>
  );
}
