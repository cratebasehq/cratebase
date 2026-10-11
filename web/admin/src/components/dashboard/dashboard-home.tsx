import { useMemo } from "react";
import { useQueries, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Activity, ArrowUpRight, Boxes, Database, Plus, ShieldUser } from "lucide-react";
import type { CollectionModel } from "@cratebase/client";
import { cb, parseServerDate } from "@/lib/api";
import { cn } from "@/lib/utils";
import { fillHourlyBuckets, type LogBucket } from "@/lib/log-stats";
import { userFields } from "@/lib/field-types";
import { useCollections } from "@/hooks/use-collections";
import { useSettings } from "@/hooks/use-settings";
import { useConnectionStatus } from "@/components/layout/connection-status";
import { useShellActions } from "@/components/layout/app-shell";
import { OnboardingChecklist } from "@/components/dashboard/onboarding-checklist";
import { PageHeader, PageSection, PageShell } from "@/components/layout/page";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { StatCard, StatusBadge, StatusDot, type Tone } from "@/components/layout/stat-card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

/** A request-log row, narrowed from the SDK's loosely typed `LogModel` to
 * the fields this screen renders. */
interface LogRow {
  id: string;
  level: number;
  message: string;
  created: string;
  data?: { method?: string; url?: string; status?: number; execTime?: number; error?: string };
}

/** What a collection's rules say about who can read it, in three words. */
function accessOf(collection: CollectionModel): { label: string; tone: "open" | "rules" | "locked" } {
  const rules = [collection.listRule, collection.viewRule];
  if (rules.every((r) => r === null || r === undefined)) return { label: "superusers", tone: "locked" };
  if (rules.some((r) => r === "")) return { label: "public", tone: "open" };
  return { label: "by rule", tone: "rules" };
}

function statusTone(status: number | undefined): Tone {
  if (status === undefined) return "neutral";
  if (status >= 500) return "danger";
  if (status >= 400) return "warning";
  return "success";
}

const AGO = new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style: "narrow" });

/** "3 min. ago" for a server timestamp. Rows refresh with their query, so no
 * ticking clock is needed. */
function timeAgo(created: string): string {
  const seconds = Math.round((parseServerDate(created).getTime() - Date.now()) / 1000);
  const abs = Math.abs(seconds);
  if (abs < 60) return AGO.format(seconds, "second");
  if (abs < 3600) return AGO.format(Math.round(seconds / 60), "minute");
  if (abs < 86400) return AGO.format(Math.round(seconds / 3600), "hour");
  return AGO.format(Math.round(seconds / 86400), "day");
}

/**
 * Requests per hour for the last day, as a bar per bucket.
 *
 * Deliberately not a charting library: 24 bars with a hover readout is a
 * flexbox and a tooltip, and a chart runtime would cost more than the whole
 * rest of this screen. When something needs axes and several series, that is
 * the moment for the library.
 */
function RequestsChart({ buckets }: { buckets: LogBucket[] }) {
  const peak = Math.max(1, ...buckets.map((b) => b.total));
  return (
    <div className="flex h-24 items-end gap-0.5" role="img" aria-label={`Requests per hour, peak ${peak}`}>
      {buckets.map((bucket, i) => {
        const hour = new Date(bucket.date);
        const height = bucket.total === 0 ? 2 : Math.max(4, Math.round((bucket.total / peak) * 100));
        return (
          <Tooltip key={bucket.date}>
            <TooltipTrigger asChild>
              <div className="flex h-full flex-1 items-end">
                <div
                  style={{ height: `${height}%`, "--i": i } as React.CSSProperties}
                  className={cn(
                    "animate-rise w-full origin-bottom rounded-t-[3px] transition-colors",
                    bucket.total === 0 ? "bg-border" : "bg-primary/60 hover:bg-primary",
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

function CardLink({ to, children }: { to: string; children: React.ReactNode }) {
  return (
    <Link
      to={to as never}
      className="flex items-center gap-1 rounded-sm text-xs text-muted-foreground transition-colors hover:text-foreground"
    >
      {children}
      <ArrowUpRight className="size-3" />
    </Link>
  );
}

/**
 * The home screen. It answers the three questions someone opening a backend
 * dashboard actually has: what is in here, is it being used, and is
 * anything broken.
 */
export function DashboardHome() {
  const { data: collections, isPending: collectionsPending } = useCollections();
  const { openNewCollection, openSearch } = useShellActions();
  const { data: settings } = useSettings();
  const { state, latencyMs } = useConnectionStatus();

  const userCollections = useMemo(() => (collections ?? []).filter((c) => !c.system), [collections]);

  // One cheap `perPage=1` count per collection — the total is the only part
  // of the response this needs.
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

  const recent = useQuery({
    queryKey: ["home-recent-requests"],
    queryFn: async () => {
      const page = await cb.admin.logs.list({ page: 1, perPage: 40, sort: "-created", skipTotal: true });
      return page.items as unknown as LogRow[];
    },
    staleTime: 15_000,
    refetchInterval: 30_000,
    retry: false,
  });

  // Job queue is an optional module: when it is off the list endpoint fails,
  // and the card simply does not appear. Gated on the setting so a disabled
  // module is never probed (a 404 here would show up as a failed request).
  const queue = useQuery({
    queryKey: ["home-queue"],
    queryFn: async () => {
      const count = async (status: string) =>
        (await cb.collection("_queue_jobs").list({ page: 1, perPage: 1, filter: `status = "${status}"` })).totalItems;
      const [pending, running, failed] = await Promise.all([count("pending"), count("in_progress"), count("failed")]);
      return { pending, running, failed };
    },
    staleTime: 30_000,
    refetchInterval: 30_000,
    retry: false,
    enabled: settings?.queue.enabled === true,
  });

  const buckets = useMemo(() => fillHourlyBuckets(stats.data ?? []), [stats.data]);
  const requests24h = buckets.reduce((sum, b) => sum + b.total, 0);

  const rows = recent.data ?? [];
  const dayAgo = Date.now() - 24 * 3600_000;
  const failures24h = rows.filter((r) => r.level >= 8 && parseServerDate(r.created).getTime() > dayAgo).length;
  const shownRows = rows.slice(0, 8);

  const origin = cb.buildURL("/") === "/" ? window.location.origin : cb.buildURL("/");
  const serverTone: Tone = state === "connected" ? "success" : state === "connecting" ? "neutral" : state === "degraded" ? "warning" : "danger";
  const serverLabel =
    state === "connected" ? "Healthy" : state === "connecting" ? "Checking…" : state === "degraded" ? "Not answering" : "Offline";

  return (
    <PageShell width="wide">
      <PageHeader
        title="Overview"
        description="What's in this backend, whether it's being used, and whether anything is broken."
        meta={
          <>
            <span className="flex items-center gap-1.5">
              <StatusDot tone={serverTone} pulse={state === "connecting"} />
              <span className="font-mono">{origin}</span>
            </span>
            {latencyMs !== null && state === "connected" ? <span className="font-tabular">{latencyMs} ms</span> : null}
          </>
        }
        actions={
          <Button size="sm" onClick={openNewCollection}>
            <Plus />
            New collection
          </Button>
        }
      />

      <OnboardingChecklist collections={collections} />

      <section className="grid grid-cols-2 gap-2 lg:grid-cols-4 xl:grid-cols-5" aria-label="At a glance">
        <StatCard
          index={0}
          label="Collections"
          value={userCollections.length}
          hint={`${(collections ?? []).length - userCollections.length} system`}
          loading={collectionsPending}
          icon={<Database />}
        />
        <StatCard
          index={1}
          label="Records"
          value={totalRecords.toLocaleString()}
          hint="across all collections"
          loading={!countsSettled}
          icon={<Boxes />}
        />
        <StatCard
          index={2}
          label="Requests · 24h"
          value={stats.isError ? "—" : requests24h.toLocaleString()}
          hint={`peak ${Math.max(0, ...buckets.map((b) => b.total)).toLocaleString()} / hour`}
          loading={stats.isPending}
          icon={<Activity />}
          to="/settings/logs"
        />
        <StatCard
          index={3}
          label="Failures · 24h"
          value={recent.isError ? "—" : failures24h}
          hint={recent.isSuccess && rows.length >= 40 ? "of the last 40 requests" : "5xx and errors"}
          tone={failures24h > 0 ? "danger" : "neutral"}
          loading={recent.isPending}
          to="/settings/logs"
        />
        {queue.data ? (
          <StatCard
            index={4}
            label="Queue"
            value={queue.data.pending + queue.data.running}
            hint={queue.data.failed > 0 ? `${queue.data.failed} failed` : "pending + running"}
            tone={queue.data.failed > 0 ? "warning" : "neutral"}
            to="/settings/automation"
            search={{ tab: "queue" }}
            className="col-span-2 lg:col-span-1"
          />
        ) : null}
      </section>

      <div className="grid gap-section lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <PageSection
          title="Requests per hour"
          description="Last 24 hours, UTC hour buckets"
          actions={<CardLink to="/settings/logs">Request logs</CardLink>}
          bordered
        >
          {stats.isPending ? (
            <Skeleton className="h-24 w-full" />
          ) : stats.isError ? (
            <ErrorState compact title="Log stats unavailable" error="The stats endpoint didn't answer." onRetry={() => void stats.refetch()} />
          ) : (
            <>
              <RequestsChart buckets={buckets} />
              <div className="mt-1.5 flex justify-between font-tabular text-2xs text-muted-foreground">
                <span>24h ago</span>
                <span>now</span>
              </div>
            </>
          )}
        </PageSection>

        <PageSection title="Server" bordered>
          <dl className="grid grid-cols-[auto_1fr] items-center gap-x-4 gap-y-2 text-sm">
            <dt className="text-muted-foreground">Status</dt>
            <dd className="justify-self-end">
              <StatusBadge tone={serverTone}>{serverLabel}</StatusBadge>
            </dd>
            <dt className="text-muted-foreground">Health check</dt>
            <dd className="justify-self-end font-tabular">{latencyMs !== null && state === "connected" ? `${latencyMs} ms` : "—"}</dd>
            {queue.data ? (
              <>
                <dt className="text-muted-foreground">Queue</dt>
                <dd className="flex items-center gap-1.5 justify-self-end font-tabular">
                  {queue.data.failed > 0 ? (
                    <StatusBadge tone="warning">{queue.data.failed} failed</StatusBadge>
                  ) : (
                    <StatusBadge tone="success">{queue.data.pending + queue.data.running} active</StatusBadge>
                  )}
                </dd>
              </>
            ) : null}
            <dt className="text-muted-foreground">Quick jump</dt>
            <dd className="justify-self-end">
              <button
                type="button"
                onClick={openSearch}
                className="flex items-center gap-1 rounded-sm text-xs text-muted-foreground transition-colors hover:text-foreground"
              >
                <Kbd>⌘K</Kbd> to search anything
              </button>
            </dd>
          </dl>
        </PageSection>
      </div>

      <div className="grid gap-section lg:grid-cols-2 lg:items-start">
        <PageSection title="Collections">
          {collectionsPending ? (
            <ListSkeleton rows={3} />
          ) : userCollections.length === 0 ? (
            <EmptyState
              compact
              icon={<Boxes />}
              title="No collections yet"
              description="A collection is a table with an API. Create one and it is readable and writable over REST straight away."
              action={
                <Button size="sm" onClick={openNewCollection}>
                  <Plus />
                  New collection
                </Button>
              }
            />
          ) : (
            <div className="overflow-hidden rounded-lg border border-border bg-card">
              <div className="grid grid-cols-[1fr_3.5rem_4.5rem_5.5rem] items-center gap-2 border-b border-border bg-surface-sunken/60 px-3 py-1.5 text-2xs tracking-wider text-muted-foreground uppercase">
                <span>Name</span>
                <span className="text-right">Fields</span>
                <span className="text-right">Records</span>
                <span className="text-right">Read</span>
              </div>
              {userCollections.map((collection, i) => {
                const access = accessOf(collection);
                return (
                  <Link
                    key={collection.id}
                    to="/collections/$name"
                    params={{ name: collection.name }}
                    className="grid grid-cols-[1fr_3.5rem_4.5rem_5.5rem] items-center gap-2 border-b border-border px-3 py-1.5 transition-colors last:border-b-0 hover:bg-accent/50 focus-visible:bg-accent/50"
                  >
                    <span className="flex min-w-0 items-center gap-1.5">
                      {collection.type === "auth" ? (
                        <ShieldUser className="size-3.5 shrink-0 text-muted-foreground" />
                      ) : (
                        <Database className="size-3.5 shrink-0 text-muted-foreground" />
                      )}
                      <span className="truncate font-mono text-sm">{collection.name}</span>
                    </span>
                    <span className="text-right font-tabular text-sm text-muted-foreground">{userFields(collection).length}</span>
                    <span className="text-right font-tabular text-sm">
                      {counts[i]?.isPending ? "—" : (counts[i]?.data ?? 0).toLocaleString()}
                    </span>
                    <span className="flex justify-end">
                      <Badge
                        variant="outline"
                        className={cn(
                          "font-normal",
                          access.tone === "open" && "text-success",
                          access.tone === "locked" && "text-muted-foreground",
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
        </PageSection>

        <PageSection title="Recent requests" actions={<CardLink to="/settings/logs">All logs</CardLink>}>
          {recent.isPending ? (
            <ListSkeleton rows={5} />
          ) : recent.isError ? (
            <ErrorState compact title="Couldn't load requests" error={recent.error} onRetry={() => void recent.refetch()} />
          ) : shownRows.length === 0 ? (
            <EmptyState compact icon={<Activity />} title="No requests yet" description="Calls to /api/* show up here as they happen." />
          ) : (
            <div className="overflow-hidden rounded-lg border border-border bg-card">
              {shownRows.map((row) => {
                const path = row.data?.url ? new URL(row.data.url, window.location.origin).pathname : row.message;
                return (
                  <div key={row.id} className="flex items-center gap-2 border-b border-border px-3 py-1.5 last:border-b-0">
                    <StatusDot tone={statusTone(row.data?.status)} />
                    <span className="w-8 shrink-0 font-mono text-2xs text-muted-foreground">{row.data?.status ?? "err"}</span>
                    <span className="w-9 shrink-0 font-mono text-2xs text-muted-foreground">{row.data?.method}</span>
                    <span className="min-w-0 flex-1 truncate font-mono text-xs" title={row.data?.url ?? row.message}>
                      {path}
                    </span>
                    <span className="shrink-0 font-tabular text-2xs text-muted-foreground">{timeAgo(row.created)}</span>
                  </div>
                );
              })}
            </div>
          )}
        </PageSection>
      </div>
    </PageShell>
  );
}
