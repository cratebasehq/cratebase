import { useMemo } from "react";
import { useQueries, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ArrowUpRight, Boxes, Database, Plus, ShieldUser, TriangleAlert } from "lucide-react";
import type { CollectionModel } from "@cratebase/client";
import { cb } from "@/lib/api";
import { cn } from "@/lib/utils";
import { fillHourlyBuckets, type LogBucket } from "@/lib/log-stats";
import { userFields } from "@/lib/field-types";
import { useCollections } from "@/hooks/use-collections";
import { useConnectionStatus } from "@/components/layout/connection-status";
import { useShellActions } from "@/components/layout/app-shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

/** A failing log row, narrowed from the SDK's `LogModel` (whose `level` is
 * typed loosely) to the handful of fields this screen renders. */
interface LogRow {
  id: string;
  message: string;
  created: string;
  data?: { method?: string; url?: string; status?: number; error?: string };
}

/** System collections are the server's own plumbing — they have a REST
 * surface but nobody browses them from a home screen. */
function isUserCollection(collection: CollectionModel): boolean {
  return !collection.system;
}

/** What a collection's rules say about who can read it, in three words. */
function accessOf(collection: CollectionModel): { label: string; tone: "open" | "rules" | "locked" } {
  const rules = [collection.listRule, collection.viewRule];
  if (rules.every((r) => r === null || r === undefined)) return { label: "superusers", tone: "locked" };
  if (rules.some((r) => r === "")) return { label: "public", tone: "open" };
  return { label: "by rule", tone: "rules" };
}

function StatTile({
  label,
  value,
  hint,
  tone = "default",
}: {
  label: string;
  value: string;
  hint?: string;
  tone?: "default" | "destructive";
}) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5 rounded-lg border border-border bg-card px-3 py-2.5">
      <span className="text-2xs uppercase tracking-wider text-muted-foreground/70">{label}</span>
      <span
        className={cn(
          "font-tabular text-2xl leading-none tracking-tight",
          tone === "destructive" && value !== "0" && "text-destructive",
        )}
      >
        {value}
      </span>
      {hint ? <span className="truncate text-2xs text-muted-foreground">{hint}</span> : null}
    </div>
  );
}

/**
 * Requests per hour for the last day, as a bar per bucket.
 *
 * Deliberately not a charting library: 24 bars with a hover readout is a
 * flexbox and a title attribute, and pulling in a full chart runtime for it
 * would cost more than the whole rest of this screen. When something needs
 * axes and multiple series, that is the moment for the library.
 */
function RequestsChart({ buckets }: { buckets: LogBucket[] }) {
  const peak = Math.max(1, ...buckets.map((b) => b.total));
  return (
    <div className="flex h-20 items-end gap-px">
      {buckets.map((bucket) => {
        const hour = new Date(bucket.date);
        const height = bucket.total === 0 ? 2 : Math.max(3, Math.round((bucket.total / peak) * 100));
        return (
          <Tooltip key={bucket.date}>
            <TooltipTrigger asChild>
              <div className="flex h-full flex-1 items-end">
                <div
                  style={{ height: `${height}%` }}
                  className={cn(
                    "w-full rounded-t-[2px] transition-colors",
                    bucket.total === 0 ? "bg-border" : "bg-foreground/70 hover:bg-foreground",
                  )}
                />
              </div>
            </TooltipTrigger>
            <TooltipContent side="top">
              <p className="font-tabular">
                {bucket.total.toLocaleString()} {bucket.total === 1 ? "request" : "requests"}
              </p>
              <p className="font-tabular opacity-70">
                {hour.toLocaleDateString(undefined, { month: "short", day: "numeric" })}{" "}
                {hour.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}
              </p>
            </TooltipContent>
          </Tooltip>
        );
      })}
    </div>
  );
}

/**
 * The home screen, which used to be an empty-state illustration.
 *
 * It answers the three questions someone opening a backend dashboard
 * actually has: what is in here, is it being used, and is anything broken.
 */
export function DashboardHome() {
  const { data: collections, isPending: collectionsPending } = useCollections();
  const { openNewCollection } = useShellActions();
  const { state, latencyMs } = useConnectionStatus();

  const userCollections = useMemo(
    () => (collections ?? []).filter(isUserCollection),
    [collections],
  );

  // One cheap `perPage=1` count per collection — the total is the only part
  // of the response this needs, and every one is cached under the same key
  // the collection page would use for its own first page.
  const counts = useQueries({
    queries: userCollections.map((collection) => ({
      queryKey: ["record-count", collection.name],
      queryFn: async () => (await cb.collection(collection.name).list({ page: 1, perPage: 1 })).totalItems,
      staleTime: 60_000,
    })),
  });

  const totalRecords = counts.reduce((sum, q) => sum + (q.data ?? 0), 0);
  const countsSettled = counts.every((q) => !q.isPending);

  const stats = useQuery({
    queryKey: ["log-stats"],
    queryFn: () => cb.admin.logs.stats() as Promise<LogBucket[]>,
    staleTime: 60_000,
    retry: false,
  });

  const failures = useQuery({
    queryKey: ["log-failures"],
    queryFn: async () => {
      const page = await cb.admin.logs.list({ page: 1, perPage: 6, filter: "level >= 8", sort: "-created", skipTotal: true });
      return page.items.map((item) => ({
        id: item.id,
        message: item.message,
        created: item.created,
        data: item.data as LogRow["data"],
      }));
    },
    staleTime: 30_000,
    retry: false,
  });

  // The stats endpoint returns every bucket it has; the home screen shows a
  // day of them, zero-filled so an idle hour is a gap rather than a missing
  // bar that silently compresses the timeline.
  const buckets = useMemo(() => fillHourlyBuckets(stats.data ?? []), [stats.data]);

  const requests24h = buckets.reduce((sum, b) => sum + b.total, 0);
  const failureRows: LogRow[] = failures.data ?? [];
  const recentFailures = failureRows.filter(
    (row) => new Date(row.created).getTime() > Date.now() - 24 * 3600_000,
  ).length;

  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col gap-section overflow-y-auto p-page">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div className="flex flex-col">
          <h1 className="text-xl font-semibold tracking-tight">Overview</h1>
          <p className="text-sm text-muted-foreground">
            {cb.buildURL("/") === "/" ? window.location.origin : cb.buildURL("/")} ·{" "}
            {state === "connected"
              ? `answering in ${latencyMs ?? 0} ms`
              : state === "connecting"
                ? "checking…"
                : state === "degraded"
                  ? "not answering"
                  : "offline"}
          </p>
        </div>
        <Button size="sm" className="gap-1.5" onClick={openNewCollection}>
          <Plus className="size-3.5" />
          New collection
        </Button>
      </header>

      <section className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        <StatTile
          label="Collections"
          value={collectionsPending ? "—" : String(userCollections.length)}
          hint={`${(collections ?? []).length - userCollections.length} system`}
        />
        <StatTile
          label="Records"
          value={countsSettled ? totalRecords.toLocaleString() : "—"}
          hint="across all collections"
        />
        <StatTile
          label="Requests"
          value={stats.isError ? "—" : requests24h.toLocaleString()}
          hint="last 24 hours"
        />
        <StatTile
          label="Failures"
          value={failures.isError ? "—" : String(recentFailures)}
          hint="last 24 hours"
          tone="destructive"
        />
      </section>

      <section className="flex flex-col gap-2 rounded-lg border border-border bg-card p-3">
        <div className="flex items-baseline justify-between gap-2">
          <h2 className="text-sm font-medium">Requests per hour</h2>
          <Link
            to="/settings/logs"
            className="flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
          >
            Request logs
            <ArrowUpRight className="size-3" />
          </Link>
        </div>
        {stats.isPending ? (
          <Skeleton className="h-20 w-full" />
        ) : stats.isError ? (
          <p className="py-6 text-center text-sm text-muted-foreground">
            The log stats endpoint didn't answer.
          </p>
        ) : (
          <>
            <RequestsChart buckets={buckets} />
            <div className="flex justify-between font-tabular text-2xs text-muted-foreground/70">
              <span>24h ago</span>
              <span>now</span>
            </div>
          </>
        )}
      </section>

      <div className="grid gap-section lg:grid-cols-2 lg:items-start">
      <section className="flex flex-col gap-2">
        <h2 className="text-sm font-medium">Collections</h2>
        {collectionsPending ? (
          <div className="flex flex-col gap-1">
            {Array.from({ length: 3 }).map((_, i) => (
              <Skeleton key={i} className="h-row w-full" />
            ))}
          </div>
        ) : userCollections.length === 0 ? (
          <div className="flex flex-col items-center gap-2 rounded-lg border border-dashed border-border px-3 py-10 text-center">
            <Boxes className="size-5 text-muted-foreground" />
            <p className="text-sm font-medium">No collections yet</p>
            <p className="max-w-measure text-sm text-muted-foreground">
              A collection is a table with an API. Create one and it is readable and writable over REST straight
              away.
            </p>
            <Button size="sm" className="mt-1 gap-1.5" onClick={openNewCollection}>
              <Plus className="size-3.5" />
              New collection
            </Button>
          </div>
        ) : (
          <div className="overflow-hidden rounded-lg border border-border">
            <div className="grid grid-cols-[1fr_5rem_5rem_6rem] items-center gap-2 border-b border-border bg-surface-sunken/60 px-3 py-1.5 text-2xs uppercase tracking-wider text-muted-foreground/70">
              <span>Name</span>
              <span className="text-right">Fields</span>
              <span className="text-right">Records</span>
              <span className="text-right">Read access</span>
            </div>
            {userCollections.map((collection: CollectionModel, i: number) => {
              const access = accessOf(collection);
              return (
                <Link
                  key={collection.id}
                  to="/collections/$name"
                  params={{ name: collection.name }}
                  className="grid grid-cols-[1fr_5rem_5rem_6rem] items-center gap-2 border-b border-border px-3 py-1.5 transition-colors last:border-b-0 hover:bg-accent/40"
                >
                  <span className="flex min-w-0 items-center gap-1.5">
                    {collection.type === "auth" ? (
                      <ShieldUser className="size-3.5 shrink-0 text-muted-foreground" />
                    ) : (
                      <Database className="size-3.5 shrink-0 text-muted-foreground" />
                    )}
                    <span className="truncate font-mono text-sm">{collection.name}</span>
                  </span>
                  <span className="text-right font-tabular text-sm text-muted-foreground">
                    {userFields(collection).length}
                  </span>
                  <span className="text-right font-tabular text-sm">
                    {counts[i]?.isPending ? "—" : (counts[i]?.data ?? 0).toLocaleString()}
                  </span>
                  <span className="flex justify-end">
                    <Badge
                      variant="outline"
                      className={cn(
                        "font-normal",
                        access.tone === "open" && "text-success",
                        access.tone === "locked" && "text-warning",
                      )}
                    >
                      {access.label}
                    </Badge>
                  </span>
                </Link>
              );
            })}
          </div>
        )}
      </section>

      {failureRows.length > 0 ? (
        <section className="flex flex-col gap-2">
          <div className="flex items-baseline justify-between gap-2">
            <h2 className="flex items-center gap-1.5 text-sm font-medium">
              <TriangleAlert className="size-3.5 text-destructive" />
              Recent failures
            </h2>
            <Link
              to="/settings/logs"
              className="flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
            >
              All logs
              <ArrowUpRight className="size-3" />
            </Link>
          </div>
          <div className="overflow-hidden rounded-lg border border-border">
            {failureRows.map((row) => (
              <div
                key={row.id}
                className="flex items-center gap-2 border-b border-border px-3 py-1.5 last:border-b-0"
              >
                <Badge variant="outline" className="shrink-0 font-mono font-normal text-destructive">
                  {row.data?.status ?? "err"}
                </Badge>
                <span className="shrink-0 font-mono text-2xs text-muted-foreground">{row.data?.method}</span>
                <span className="min-w-0 flex-1 truncate font-mono text-xs" title={row.data?.url}>
                  {row.data?.url ? new URL(row.data.url, window.location.origin).pathname : row.message}
                </span>
                <span className="shrink-0 font-tabular text-2xs text-muted-foreground">
                  {new Date(row.created).toLocaleTimeString()}
                </span>
              </div>
            ))}
          </div>
        </section>
      ) : null}
      </div>
    </div>
  );
}
