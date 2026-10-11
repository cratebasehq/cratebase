import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CalendarClock, CircleAlert, Clock, History, Pause, Pencil, Play, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { cb, describeFailure, parseServerDate } from "@/lib/api";
import { describeJob, describeSchedule, isKnownSystemJob } from "@/lib/cron";
import { settingsItemFor } from "@/lib/settings-nav";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { StatCard, StatusBadge } from "@/components/layout/stat-card";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { SettingsPage } from "@/components/settings/settings-form";
import { nextCronRun } from "@/components/settings/job-support";
import { LIVE_INTERVAL_MS, LiveToggle } from "@/components/settings/live-toggle";
import { TimeAgo, relativeTime } from "@/components/settings/time-ago";
import { TypedConfirmDialog } from "@/components/settings/typed-confirm-dialog";
import {
  DetailBody,
  DetailCallout,
  DetailEmpty,
  DetailGrid,
  DetailGridItem,
  DetailList,
  DetailRow,
  DetailSection,
  DetailSheet,
  DetailSheetHeader,
  DetailTabPanel,
  DetailTabs,
  DetailTimeline,
  DetailListItem,
} from "@/components/layout/detail-sheet";
import { CodeBlock } from "@/components/settings/detail-json";

/** As `GET /api/crons` returns them. */
type CronJob = {
  id: string;
  expression: string;
};

/** A `_cron_jobs` record — a user-defined job that runs a SQL statement on
 * a schedule. `id`s from `GET /api/crons` for these are `custom:<record
 * id>`, which is how "Run now" below reaches the same scheduler entry a
 * dashboard edit registers. */
interface CustomCronJob {
  id: string;
  name: string;
  expression: string;
  sql: string;
  enabled: boolean;
  lastRunAt?: string;
  lastStatus?: string;
  lastMessage?: string;
}

/** A `_cronRuns` record — see `crates/server/src/cron_history.rs`. */
interface CronRun {
  id: string;
  jobId: string;
  source: "sql" | "js";
  status: "success" | "error";
  startedAt: string;
  durationMs?: number;
  message?: string;
}

type Source = "system" | "code" | "sql";
type SourceFilter = Source | "all";

const SOURCE_LABEL: Record<Source, string> = { system: "System", code: "Code", sql: "SQL" };

/** One row of the unified schedule: a built-in job, a `cronAdd` registration,
 * or a `_cron_jobs` record. */
interface JobRow {
  id: string;
  source: Source;
  title: string;
  detail: string;
  expression: string;
  enabled: boolean;
  custom?: CustomCronJob;
  last: { at: Date; status: "success" | "error"; message?: string; durationMs?: number } | null;
  next: Date | null;
}

/**
 * Everything the scheduler runs, in one table: the **system** jobs registered
 * in code (log trimming, expired codes, database upkeep, the scheduled
 * backup), **code** jobs a `cb_hooks/*.cb.js` file registered with `cronAdd`,
 * and **SQL** jobs — ordinary `_cron_jobs` records with full CRUD. All three
 * share the same `POST /api/crons/{id}` "run now" endpoint; a SQL job's id
 * there is `custom:<record id>`. The view refreshes itself every few seconds.
 */
export function CronJobsPage() {
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [source, setSource] = useState<SourceFilter>("all");
  const [dialogJob, setDialogJob] = useState<CustomCronJob | "new" | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<CustomCronJob | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [tab, setTab] = useState("overview");

  const dialogOpen = dialogJob !== null || deleteTarget !== null;
  const [live, setLive] = useState(true);
  // Hold still while a dialog is open so rows don't shift under the operator.
  const refetchInterval = live && !dialogOpen ? LIVE_INTERVAL_MS : false;

  const crons = useQuery({
    queryKey: ["crons"],
    queryFn: () => cb.send<CronJob[]>("/api/crons", { method: "GET" }),
    refetchInterval,
  });

  const custom = useQuery({
    queryKey: ["custom-cron-jobs"],
    queryFn: () => cb.collection("_cron_jobs").fullList({ sort: "-created" }) as unknown as Promise<CustomCronJob[]>,
    refetchInterval,
  });

  // Latest run per job id, from the most recent runs across every job. A job
  // that hasn't fired lately simply has no entry.
  const latestRuns = useQuery({
    queryKey: ["cron-runs-latest"],
    queryFn: () =>
      cb.collection("_cronRuns").list({
        page: 1,
        perPage: 200,
        sort: "-startedAt",
      }) as unknown as Promise<{ items: CronRun[] }>,
    refetchInterval,
  });

  function refreshAll() {
    void queryClient.invalidateQueries({ queryKey: ["crons"] });
    void queryClient.invalidateQueries({ queryKey: ["custom-cron-jobs"] });
    void queryClient.invalidateQueries({ queryKey: ["cron-runs-latest"] });
    void queryClient.invalidateQueries({ queryKey: ["cron-runs"] });
  }

  const run = useMutation({
    mutationFn: (id: string) => cb.send<void>(`/api/crons/${encodeURIComponent(id)}`, { method: "POST" }),
    onSuccess: (_result, id) => {
      toast.success(`Ran ${describeJob(id).title}`);
      // A job that touches the log or the database changes what the other
      // settings screens show.
      void queryClient.invalidateQueries({ queryKey: ["request-logs"] });
      void queryClient.invalidateQueries({ queryKey: ["backups"] });
      refreshAll();
    },
    onError: (failure, id) => {
      const described = describeFailure(failure);
      toast.error(`Couldn't run ${describeJob(id).title}`, { description: described.detail });
    },
  });

  const toggle = useMutation({
    mutationFn: ({ job, enabled }: { job: CustomCronJob; enabled: boolean }) =>
      cb.collection("_cron_jobs").update(job.id, { enabled }),
    onSuccess: (_result, { job, enabled }) => {
      toast.success(`${enabled ? "Enabled" : "Disabled"} ${job.name}`);
      refreshAll();
    },
    onError: (failure, { job }) => {
      const described = describeFailure(failure);
      toast.error(`Couldn't update ${job.name}`, { description: described.detail });
    },
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection("_cron_jobs").delete(id),
    onSuccess: (_result, id) => {
      toast.success(`Deleted ${deleteTarget?.name ?? "job"}`);
      setSelectedId((current) => (current === `custom:${id}` ? null : current));
      refreshAll();
      setDeleteTarget(null);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error("Couldn't delete the job", { description: described.detail });
    },
  });

  const runs = latestRuns.data?.items;
  // Not memoised: "next run" is relative to now and refreshes with each poll.
  const rows: JobRow[] = (() => {
    const latest: Record<string, CronRun> = {};
    for (const r of runs ?? []) {
      // Sorted newest first, so the first one seen per job wins.
      if (!(r.jobId in latest)) latest[r.jobId] = r;
    }
    const now = new Date();
    const out: JobRow[] = [];

    for (const job of crons.data ?? []) {
      if (job.id.startsWith("custom:")) continue;
      const described = describeJob(job.id);
      const lastRun = latest[job.id];
      out.push({
        id: job.id,
        source: isKnownSystemJob(job.id) ? "system" : "code",
        title: described.title,
        detail: described.detail,
        expression: job.expression,
        enabled: true,
        last: lastRun
          ? {
              at: parseServerDate(lastRun.startedAt),
              status: lastRun.status,
              message: lastRun.message,
              durationMs: lastRun.durationMs,
            }
          : null,
        next: nextCronRun(job.expression, now),
      });
    }

    for (const job of custom.data ?? []) {
      const id = `custom:${job.id}`;
      const lastRun = latest[id];
      let last: JobRow["last"] = null;
      if (lastRun) {
        last = {
          at: parseServerDate(lastRun.startedAt),
          status: lastRun.status,
          message: lastRun.message,
          durationMs: lastRun.durationMs,
        };
      } else if (job.lastRunAt) {
        last = {
          at: parseServerDate(job.lastRunAt),
          status: job.lastStatus === "error" ? "error" : "success",
          message: job.lastMessage,
        };
      }
      out.push({
        id,
        source: "sql",
        title: job.name,
        detail: job.sql,
        expression: job.expression,
        enabled: job.enabled,
        custom: job,
        last,
        next: job.enabled ? nextCronRun(job.expression, now) : null,
      });
    }
    return out;
  })();

  const needle = search.trim().toLowerCase();
  const visible = rows.filter((row) => {
    if (source !== "all" && row.source !== source) return false;
    if (!needle) return true;
    return [row.title, row.id, row.expression, row.detail].some((text) => text.toLowerCase().includes(needle));
  });
  const selected = selectedId ? rows.find((row) => row.id === selectedId) : undefined;
  function openJob(id: string, nextTab = "overview") {
    setSelectedId(id);
    setTab(nextTab);
  }

  const failing = rows.filter((row) => row.last?.status === "error").length;
  const disabled = rows.filter((row) => !row.enabled).length;
  const soonest = rows.reduce<JobRow | null>((best, row) => {
    if (!row.next) return best;
    return !best || !best.next || row.next < best.next ? row : best;
  }, null);
  const counts = {
    system: rows.filter((row) => row.source === "system").length,
    code: rows.filter((row) => row.source === "code").length,
    sql: rows.filter((row) => row.source === "sql").length,
  };

  const loading = crons.isPending;
  const runningId = run.isPending ? (run.variables ?? null) : null;
  const filtering = needle !== "" || source !== "all";
  const fetching = crons.isFetching || custom.isFetching || latestRuns.isFetching;

  const item = settingsItemFor("/settings/cron")!;
  return (
    <SettingsPage title={item.label} description={item.description} width="wide">
      <div className="grid grid-cols-2 gap-2 lg:grid-cols-4">
        <StatCard
          index={0}
          label="Scheduled jobs"
          icon={<Clock />}
          loading={loading}
          value={rows.length}
          hint={`${counts.system} system · ${counts.code} code · ${counts.sql} SQL`}
        />
        <StatCard
          index={1}
          label="Next run"
          icon={<CalendarClock />}
          loading={loading}
          value={soonest?.next ? <time dateTime={soonest.next.toISOString()} title={soonest.next.toLocaleString()}>{relativeTime(soonest.next).replace(/\.$/, "")}</time> : "—"}
          hint={soonest ? soonest.title : "Nothing scheduled"}
        />
        <StatCard
          index={2}
          label="Last run failed"
          icon={<CircleAlert />}
          loading={loading || latestRuns.isPending}
          value={failing}
          tone={failing > 0 ? "danger" : "neutral"}
          hint={failing > 0 ? "Open history on a job to see why" : "All recent runs succeeded"}
        />
        <StatCard
          index={3}
          label="Disabled"
          loading={loading || custom.isPending}
          value={disabled}
          hint={disabled > 0 ? "SQL jobs that won't fire" : "Everything is enabled"}
        />
      </div>

      <DataTableToolbar
        search={{ value: search, onChange: setSearch, placeholder: "Search jobs…", label: "Search jobs" }}
        filters={
          <Select value={source} onValueChange={(v) => setSource(v as SourceFilter)}>
            <SelectTrigger size="sm" aria-label="Filter by source" className="w-36">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All sources</SelectItem>
              <SelectItem value="system">System</SelectItem>
              <SelectItem value="code">Code (cronAdd)</SelectItem>
              <SelectItem value="sql">SQL</SelectItem>
            </SelectContent>
          </Select>
        }
        actions={
          <>
            <LiveToggle live={live} onLiveChange={setLive} fetching={fetching} onRefresh={refreshAll} />
            <Button size="sm" onClick={() => setDialogJob("new")}>
              <Plus />
              New SQL job
            </Button>
          </>
        }
      />

      {crons.isError ? (
        <ErrorState
          title="Couldn't load the schedule"
          error={crons.error}
          onRetry={() => void crons.refetch()}
          compact
        />
      ) : loading ? (
        <ListSkeleton rows={5} />
      ) : (
        <>
          {custom.isError ? (
            <ErrorState
              title="Couldn't load SQL jobs"
              error={custom.error}
              onRetry={() => void custom.refetch()}
              compact
            />
          ) : null}
          {visible.length === 0 ? (
            filtering ? (
              <EmptyState
                compact
                icon={<Clock />}
                title="No jobs match"
                description={
                  source === "code" && !needle ? (
                    <>
                      No hook file has registered one yet. Call <code className="font-mono">cronAdd(id, "*/5 * * * *", fn)</code>{" "}
                      from a <code className="font-mono">cb_hooks/*.cb.js</code> file and restart the server.
                    </>
                  ) : source === "sql" && !needle ? (
                    "Add one to run a SQL statement on a schedule."
                  ) : (
                    "Try a different search or source."
                  )
                }
                action={
                  source === "sql" && !needle ? (
                    <Button size="sm" onClick={() => setDialogJob("new")}>
                      <Plus />
                      New SQL job
                    </Button>
                  ) : (
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => {
                        setSearch("");
                        setSource("all");
                      }}
                    >
                      Clear filters
                    </Button>
                  )
                }
              />
            ) : (
              <EmptyState
                compact
                icon={<Clock />}
                title="No scheduled jobs"
                description="Nothing is registered on this server. Add a SQL job here, or register one from a hook file with cronAdd."
                action={
                  <Button size="sm" onClick={() => setDialogJob("new")}>
                    <Plus />
                    New SQL job
                  </Button>
                }
              />
            )
          ) : (
            <div className="rounded-lg border border-border bg-card overflow-hidden">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Job</TableHead>
                    <TableHead>Schedule (UTC)</TableHead>
                    <TableHead>Last run</TableHead>
                    <TableHead>Next run</TableHead>
                    <TableHead className="w-20">Enabled</TableHead>
                    <TableHead className="w-40 text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {visible.map((row) => {
                    const running = runningId === row.id;
                    const schedule = describeSchedule(row.expression);
                    const customJob = row.custom;
                    return (
                      <TableRow key={row.id} className="align-top">
                        <TableCell className="min-w-56 py-2.5">
                          <div className="flex items-center gap-2">
                            <button
                              type="button"
                              className="rounded-sm text-left font-medium underline-offset-2 hover:underline"
                              onClick={() => openJob(row.id)}
                            >
                              {row.title}
                            </button>
                            <Badge variant="outline" className="h-4 px-1.5 text-2xs font-normal text-muted-foreground">
                              {SOURCE_LABEL[row.source]}
                            </Badge>
                          </div>
                          {row.source === "sql" ? (
                            <div className="mt-0.5 max-w-xs truncate font-mono text-xs text-muted-foreground" title={row.detail}>
                              {row.detail}
                            </div>
                          ) : (
                            <>
                              <div className="text-xs text-muted-foreground">{row.detail}</div>
                              <div className="mt-0.5 font-mono text-xs text-muted-foreground">{row.id}</div>
                            </>
                          )}
                        </TableCell>
                        <TableCell className="py-2.5">
                          <div className="text-sm">{schedule === row.expression ? "Custom schedule" : schedule}</div>
                          <div className="font-mono text-xs text-muted-foreground">{row.expression}</div>
                        </TableCell>
                        <TableCell className="py-2.5">
                          {row.last ? (
                            <>
                              <div className="flex items-center gap-2">
                                <StatusBadge tone={row.last.status === "error" ? "danger" : "success"}>
                                  {row.last.status === "error" ? "Failed" : "Succeeded"}
                                </StatusBadge>
                                <TimeAgo value={row.last.at.toISOString()} className="text-xs text-muted-foreground" />
                              </div>
                              {row.last.message || typeof row.last.durationMs === "number" ? (
                                <div
                                  className="mt-0.5 max-w-56 truncate text-xs text-muted-foreground"
                                  title={row.last.message}
                                >
                                  {typeof row.last.durationMs === "number" ? (
                                    <span className="font-tabular font-mono">{row.last.durationMs}ms</span>
                                  ) : null}
                                  {row.last.message && typeof row.last.durationMs === "number" ? " · " : null}
                                  {row.last.message}
                                </div>
                              ) : null}
                            </>
                          ) : (
                            <span className="text-xs text-muted-foreground">No run recorded</span>
                          )}
                        </TableCell>
                        <TableCell className="py-2.5 text-xs">
                          {row.next ? (
                            <TimeAgo value={row.next.toISOString()} />
                          ) : (
                            <span className="text-muted-foreground">{row.enabled ? "—" : "Disabled"}</span>
                          )}
                        </TableCell>
                        <TableCell className="py-2.5">
                          {customJob ? (
                            <Switch
                              size="sm"
                              checked={customJob.enabled}
                              disabled={toggle.isPending && toggle.variables?.job.id === customJob.id}
                              aria-label={`${customJob.enabled ? "Disable" : "Enable"} ${customJob.name}`}
                              onCheckedChange={(enabled) => toggle.mutate({ job: customJob, enabled })}
                            />
                          ) : (
                            <span className="text-xs text-muted-foreground">Always</span>
                          )}
                        </TableCell>
                        <TableCell className="py-2.5 text-right">
                          <div className="flex justify-end gap-0.5">
                            <Button
                              variant="ghost"
                              size="xs"
                              aria-label={`Run ${row.title} now`}
                              disabled={running}
                              onClick={() => run.mutate(row.id)}
                            >
                              {running ? <Spinner className="size-3" /> : <Play />}
                              {running ? "Running…" : "Run now"}
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon-xs"
                              aria-label={`View history for ${row.title}`}
                              onClick={() => openJob(row.id, "runs")}
                            >
                              <History />
                            </Button>
                            {customJob ? (
                              <>
                                <Button
                                  variant="ghost"
                                  size="icon-xs"
                                  aria-label={`Edit ${customJob.name}`}
                                  onClick={() => openJob(row.id, "settings")}
                                >
                                  <Pencil />
                                </Button>
                                <Button
                                  variant="ghost"
                                  size="icon-xs"
                                  aria-label={`Delete ${customJob.name}`}
                                  onClick={() => setDeleteTarget(customJob)}
                                >
                                  <Trash2 />
                                </Button>
                              </>
                            ) : null}
                          </div>
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </div>
          )}

          <p className="max-w-measure text-xs text-muted-foreground">
            System jobs are built in; running one executes it immediately without moving its schedule. SQL jobs run
            with no rule enforcement — the same trust tier as editing a collection's schema. Code jobs come from{" "}
            <code className="font-mono">cronAdd</code> in <code className="font-mono">cb_hooks/*.cb.js</code> and are
            edited as files.
          </p>
        </>
      )}

      {dialogJob ? (
        <CustomCronJobForm
          job={dialogJob === "new" ? null : dialogJob}
          onOpenChange={(open) => {
            if (!open) setDialogJob(null);
          }}
        />
      ) : null}

      {selected ? (
        <CronJobSheet
          row={selected}
          visible={visible}
          tab={tab}
          onTabChange={setTab}
          onSelect={(id) => setSelectedId(id)}
          onClose={() => setSelectedId(null)}
          onRun={(id) => run.mutate(id)}
          running={runningId === selected.id}
          onToggle={(job, enabled) => toggle.mutate({ job, enabled })}
          toggling={toggle.isPending}
          onDelete={(job) => setDeleteTarget(job)}
        />
      ) : null}

      <TypedConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => !open && setDeleteTarget(null)}
        title={`Delete ${deleteTarget?.name ?? "job"}?`}
        description="This stops the schedule immediately and removes its record. There is no undo."
        confirmValue={deleteTarget?.name || "delete"}
        confirmLabel="Delete job"
        pendingLabel="Deleting…"
        pending={remove.isPending}
        onConfirm={() => deleteTarget && remove.mutate(deleteTarget.id)}
      />
    </SettingsPage>
  );
}

/** Health shown in the table and the sheet header. */
function jobHealth(row: JobRow): { tone: "success" | "danger" | "neutral"; label: string } {
  if (!row.enabled) return { tone: "neutral", label: "Paused" };
  if (row.last?.status === "error") return { tone: "danger", label: "Failing" };
  return { tone: "success", label: "Healthy" };
}

const UTC_FORMAT = new Intl.DateTimeFormat("en-US", {
  weekday: "short",
  month: "short",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
  timeZone: "UTC",
});

function formatUtc(date: Date): string {
  return `${UTC_FORMAT.format(date)} UTC`;
}

function inRelative(date: Date): string {
  return relativeTime(date).replace(/\.$/, "");
}

/** The next `count` fire times after now; empty for syntax the reader can't parse. */
function upcomingRuns(expression: string, count = 5): Date[] {
  const out: Date[] = [];
  let from = new Date();
  for (let i = 0; i < count; i++) {
    const next = nextCronRun(expression, from);
    if (!next) break;
    out.push(next);
    from = next;
  }
  return out;
}

const SOURCE_HINT: Record<Source, string> = {
  system: "built in",
  code: "registered with cronAdd",
  sql: "_cron_jobs record",
};

const SCHEDULE_FIELDS = ["Minute", "Hour", "Day", "Month", "Weekday"];

/** Detail drawer for one scheduled job. Run history comes from `_cronRuns`:
 * `GET /api/crons` gives `custom:<id>` for a SQL `_cron_jobs` row or the raw
 * `cronAdd` id otherwise, which is exactly `_cronRuns.jobId`. Empty is
 * normal: a job that's never fired, or whose ticks all lost the multi-node
 * advisory-lock race on this node, has zero rows. */
function CronJobSheet({
  row,
  visible,
  tab,
  onTabChange,
  onSelect,
  onClose,
  onRun,
  running,
  onToggle,
  toggling,
  onDelete,
}: {
  row: JobRow;
  visible: JobRow[];
  tab: string;
  onTabChange: (tab: string) => void;
  onSelect: (id: string) => void;
  onClose: () => void;
  onRun: (id: string) => void;
  running: boolean;
  onToggle: (job: CustomCronJob, enabled: boolean) => void;
  toggling: boolean;
  onDelete: (job: CustomCronJob) => void;
}) {
  const history = useQuery({
    queryKey: ["cron-runs", row.id],
    queryFn: () =>
      cb.collection("_cronRuns").list({
        page: 1,
        perPage: 50,
        sort: "-startedAt",
        filter: `jobId = "${row.id}"`,
      }) as unknown as Promise<{ items: CronRun[]; totalItems: number }>,
  });
  const runs = history.data?.items ?? [];
  const runCount = history.data?.totalItems ?? runs.length;
  let streak = 0;
  for (const r of runs) {
    if (r.status !== "error") break;
    streak++;
  }

  const health = jobHealth(row);
  const custom = row.custom;
  const schedule = describeSchedule(row.expression);
  const scheduleText = schedule === row.expression ? "Custom schedule" : schedule;
  const fields = row.expression.trim().split(/\s+/);
  const upcoming = row.enabled ? upcomingRuns(row.expression) : [];

  const index = visible.findIndex((r) => r.id === row.id);
  const pager =
    index >= 0
      ? {
          index: index + 1,
          total: visible.length,
          hasPrev: index > 0,
          hasNext: index < visible.length - 1,
          onPrev: () => onSelect(visible[index - 1].id),
          onNext: () => onSelect(visible[index + 1].id),
        }
      : undefined;

  return (
    <DetailSheet open onOpenChange={(open) => !open && onClose()}>
      <DetailSheetHeader
        id={row.id}
        pager={pager}
        title={row.title}
        description={`${SOURCE_LABEL[row.source]} job ${row.id}, ${scheduleText}`}
        status={<StatusBadge tone={health.tone}>{health.label}</StatusBadge>}
        meta={[
          <span key="expr" className="font-mono">
            {row.expression}
          </span>,
          `${scheduleText} UTC`,
          SOURCE_LABEL[row.source],
        ]}
        actions={
          <>
            <Button size="sm" disabled={running} onClick={() => onRun(row.id)}>
              {running ? <Spinner className="size-3.5" /> : <Play />}
              {running ? "Running…" : "Run now"}
            </Button>
            {custom ? (
              <Button
                size="sm"
                variant="outline"
                disabled={toggling}
                onClick={() => onToggle(custom, !custom.enabled)}
              >
                {custom.enabled ? <Pause /> : <Play />}
                {custom.enabled ? "Pause" : "Resume"}
              </Button>
            ) : null}
          </>
        }
        menu={
          custom
            ? [
                { label: "Edit job", icon: <Pencil />, onSelect: () => onTabChange("settings") },
                { label: "Delete job", icon: <Trash2 />, destructive: true, onSelect: () => onDelete(custom) },
              ]
            : undefined
        }
      />
      <DetailTabs
        value={tab}
        onValueChange={onTabChange}
        tabs={[
          { value: "overview", label: "Overview" },
          { value: "runs", label: "Runs", count: runCount },
          { value: "settings", label: "Settings" },
        ]}
      >
        <DetailTabPanel value="overview">
          <DetailBody>
            {row.last?.status === "error" && row.enabled ? (
              <DetailCallout
                title="Last run failed"
                action={
                  <Button size="xs" variant="outline" onClick={() => onTabChange("runs")}>
                    View log
                  </Button>
                }
              >
                <span className="block">
                  {row.last.message ? (
                    <span className="font-mono break-words">{row.last.message}</span>
                  ) : (
                    "No message was recorded."
                  )}
                </span>
                <span className="mt-1 block text-xs opacity-80">
                  {streak > 1 ? `${streak} failed runs in a row · ` : null}last run <TimeAgo value={row.last.at.toISOString()} />
                </span>
              </DetailCallout>
            ) : null}
            <DetailSection title="Schedule">
              {fields.length === 5 ? (
                <DetailGrid cols={5}>
                  {SCHEDULE_FIELDS.map((label, i) => (
                    <DetailGridItem key={label} label={label}>
                      {fields[i]}
                    </DetailGridItem>
                  ))}
                </DetailGrid>
              ) : (
                <p className="m-0 font-mono text-sm">{row.expression}</p>
              )}
            </DetailSection>
            <DetailSection title="Details">
              <DetailList>
                <DetailRow label="Runs" hint="UTC">
                  {scheduleText}
                </DetailRow>
                <DetailRow label="Next run" hint={row.next ? inRelative(row.next) : undefined}>
                  {row.next ? (
                    <time dateTime={row.next.toISOString()} title={row.next.toLocaleString()} className="font-tabular">
                      {formatUtc(row.next)}
                    </time>
                  ) : (
                    <span className="text-muted-foreground">{row.enabled ? "—" : "Paused"}</span>
                  )}
                </DetailRow>
                <DetailRow
                  label="Last run"
                  hint={typeof row.last?.durationMs === "number" ? `${row.last.durationMs}ms` : undefined}
                >
                  {row.last ? (
                    <>
                      <StatusBadge tone={row.last.status === "error" ? "danger" : "success"}>
                        {row.last.status === "error" ? "Failed" : "Succeeded"}
                      </StatusBadge>
                      <TimeAgo value={row.last.at.toISOString()} />
                    </>
                  ) : (
                    <span className="text-muted-foreground">No run recorded</span>
                  )}
                </DetailRow>
                {row.last?.message && row.last.status !== "error" ? (
                  <DetailRow label="Message">
                    <span className="font-mono text-xs break-words">{row.last.message}</span>
                  </DetailRow>
                ) : null}
                <DetailRow label="Source" hint={SOURCE_HINT[row.source]}>
                  {SOURCE_LABEL[row.source]}
                </DetailRow>
                <DetailRow label="Enabled" hint={custom ? undefined : "always on"}>
                  {custom && !custom.enabled ? "No" : "Yes"}
                </DetailRow>
                {row.source !== "sql" ? (
                  <DetailRow label="Handler" hint={row.detail}>
                    <span className="font-mono text-xs">{row.id}</span>
                  </DetailRow>
                ) : null}
              </DetailList>
            </DetailSection>
            {row.source === "sql" ? (
              <DetailSection title="SQL">
                <CodeBlock plain={row.detail} copyLabel="Copy SQL" />
              </DetailSection>
            ) : null}
            <DetailSection title="Next 5 runs">
              {upcoming.length > 0 ? (
                <DetailTimeline>
                  {upcoming.map((date) => (
                    <DetailListItem key={date.getTime()} meta={inRelative(date)}>
                      <time dateTime={date.toISOString()} title={date.toLocaleString()}>
                        {formatUtc(date)}
                      </time>
                    </DetailListItem>
                  ))}
                </DetailTimeline>
              ) : (
                <p className="m-0 text-sm text-muted-foreground">
                  {row.enabled ? "Couldn't compute upcoming runs for this expression." : "Paused jobs don't fire."}
                </p>
              )}
            </DetailSection>
          </DetailBody>
        </DetailTabPanel>

        <DetailTabPanel value="runs">
          <DetailBody>
            {history.error ? (
              <ErrorState
                title="Couldn't load history"
                error={history.error}
                onRetry={() => void history.refetch()}
                compact
              />
            ) : history.isPending ? (
              <ListSkeleton rows={3} />
            ) : runs.length === 0 ? (
              <DetailEmpty
                icon={<History />}
                title="No runs recorded yet"
                description="A job that hasn't fired yet — or whose ticks ran on another node — has no history. Use Run now to record one."
              />
            ) : (
              <>
                <ul className="m-0 flex list-none flex-col divide-y divide-border p-0">
                  {runs.map((r) => (
                    <li key={r.id} className="flex flex-col gap-1 py-2 text-xs first:pt-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <StatusBadge tone={r.status === "success" ? "success" : "danger"}>
                          {r.status === "success" ? "Succeeded" : "Failed"}
                        </StatusBadge>
                        <Badge variant="outline" className="h-4 px-1.5 text-2xs font-normal uppercase">
                          {r.source}
                        </Badge>
                        <TimeAgo value={r.startedAt} className="text-muted-foreground" />
                        <span className="font-tabular font-mono text-muted-foreground">
                          {parseServerDate(r.startedAt).toLocaleString()}
                        </span>
                        {typeof r.durationMs === "number" ? (
                          <span className="font-tabular ml-auto font-mono text-muted-foreground">{r.durationMs}ms</span>
                        ) : null}
                      </div>
                      {r.message ? (
                        <div
                          className={
                            r.status === "error"
                              ? "font-mono break-words text-destructive"
                              : "font-mono break-words text-muted-foreground"
                          }
                        >
                          {r.message}
                        </div>
                      ) : null}
                    </li>
                  ))}
                </ul>
                {runCount > runs.length ? (
                  <p className="font-tabular mt-3 mb-0 text-xs text-muted-foreground">
                    Showing the newest {runs.length} of {runCount} runs.
                  </p>
                ) : null}
              </>
            )}
          </DetailBody>
        </DetailTabPanel>

        <DetailTabPanel value="settings">
          <DetailBody>
            {custom ? (
              <CustomCronJobForm key={custom.id} job={custom} inline onOpenChange={() => onTabChange("overview")} />
            ) : (
              <DetailEmpty
                icon={<Clock />}
                title={row.source === "system" ? "Built-in job" : "Registered in code"}
                description={
                  row.source === "system"
                    ? "System jobs are built in. Run one from the header; its schedule can't be changed here."
                    : "This job comes from cronAdd in a cb_hooks/*.cb.js file. Edit the file and restart the server to change it."
                }
              />
            )}
          </DetailBody>
        </DetailTabPanel>
      </DetailTabs>
    </DetailSheet>
  );
}

/** `sql = "invalid"` or an empty `expression` produces a server-side field
 * error (`crates/server/src/cron_jobs.rs`'s `on_record_create`/
 * `on_record_update` hooks reject it before the row is even written), so
 * this dialog surfaces that inline instead of a generic toast — alongside
 * the same checks done up front for an empty name, expression or SQL.
 * Create or edit one `_cron_jobs` record. `job === null` means create. */
function CustomCronJobForm({
  job,
  onOpenChange,
  inline = false,
}: {
  job: CustomCronJob | null;
  onOpenChange: (open: boolean) => void;
  /** Rendered inside a detail sheet instead of a dialog. */
  inline?: boolean;
}) {
  const queryClient = useQueryClient();
  const [name, setName] = useState(job?.name ?? "");
  const [expression, setExpression] = useState(job?.expression ?? "0 0 * * *");
  const [sql, setSql] = useState(job?.sql ?? "");
  const [enabled, setEnabled] = useState(job?.enabled ?? true);
  const [serverErrors, setServerErrors] = useState<Record<string, string>>({});
  const [submitted, setSubmitted] = useState(false);

  const save = useMutation({
    mutationFn: () => {
      const body = { name, expression, sql, enabled };
      return job ? cb.collection("_cron_jobs").update(job.id, body) : cb.collection("_cron_jobs").create(body);
    },
    onSuccess: () => {
      toast.success(job ? `Updated ${name}` : `Created ${name}`);
      void queryClient.invalidateQueries({ queryKey: ["custom-cron-jobs"] });
      void queryClient.invalidateQueries({ queryKey: ["crons"] });
      onOpenChange(false);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      setServerErrors(described.fields);
      if (Object.keys(described.fields).length === 0) {
        toast.error(job ? "Couldn't update the job" : "Couldn't create the job", { description: described.detail });
      }
    },
  });

  const clientErrors: Record<string, string> = {};
  if (name.trim() === "") clientErrors["name"] = "Give the job a name.";
  if (expression.trim() === "") clientErrors["expression"] = "Enter a cron expression, e.g. 0 0 * * *.";
  else if (expression.trim().split(/\s+/).length !== 5) {
    clientErrors["expression"] = "Use five space-separated fields: minute hour day month weekday.";
  }
  if (sql.trim() === "") clientErrors["sql"] = "Enter the SQL statement to run.";

  const errorFor = (field: string) => serverErrors[field] ?? (submitted ? clientErrors[field] : undefined);
  const next = nextCronRun(expression);
  const schedule = describeSchedule(expression);

  function clearServer(field: string) {
    if (serverErrors[field]) setServerErrors((prev) => ({ ...prev, [field]: "" }));
  }

  const formEl = (
        <form
          className="flex flex-col gap-4"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            setSubmitted(true);
            if (Object.keys(clientErrors).length > 0) return;
            setServerErrors({});
            save.mutate();
          }}
        >
          {inline ? null : (
            <DialogHeader>
              <DialogTitle>{job ? "Edit SQL job" : "New SQL job"}</DialogTitle>
              <DialogDescription>
                Runs with no rule enforcement — treat the SQL below the same as a collection schema edit.
              </DialogDescription>
            </DialogHeader>
          )}

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="cron-name">Name</Label>
            <Input
              id="cron-name"
              autoFocus
              value={name}
              aria-invalid={errorFor("name") ? true : undefined}
              aria-describedby={errorFor("name") ? "cron-name-error" : undefined}
              onChange={(e) => {
                setName(e.target.value);
                clearServer("name");
              }}
              placeholder="Nightly rollup"
            />
            {errorFor("name") ? (
              <p id="cron-name-error" role="alert" className="text-xs text-destructive">
                {errorFor("name")}
              </p>
            ) : null}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="cron-expression">Cron expression (UTC)</Label>
            <Input
              id="cron-expression"
              className="font-mono"
              value={expression}
              aria-invalid={errorFor("expression") ? true : undefined}
              aria-describedby={errorFor("expression") ? "cron-expression-error" : "cron-expression-hint"}
              onChange={(e) => {
                setExpression(e.target.value);
                clearServer("expression");
              }}
              placeholder="0 0 * * *"
            />
            {errorFor("expression") ? (
              <p id="cron-expression-error" role="alert" className="text-xs text-destructive">
                {errorFor("expression")}
              </p>
            ) : (
              <p id="cron-expression-hint" className="text-xs text-muted-foreground">
                {schedule === expression.trim() ? "Minute hour day-of-month month weekday." : schedule}
                {next ? (
                  <>
                    {" "}
                    · next run <span className="font-mono">{next.toLocaleString()}</span>
                  </>
                ) : null}
              </p>
            )}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="cron-sql">SQL</Label>
            <Textarea
              id="cron-sql"
              className="min-h-24 font-mono text-xs"
              value={sql}
              aria-invalid={errorFor("sql") ? true : undefined}
              aria-describedby={errorFor("sql") ? "cron-sql-error" : undefined}
              onChange={(e) => {
                setSql(e.target.value);
                clearServer("sql");
              }}
              spellCheck={false}
              placeholder='DELETE FROM "posts" WHERE "created" < date(&apos;now&apos;, &apos;-30 days&apos;)'
            />
            {errorFor("sql") ? (
              <p id="cron-sql-error" role="alert" className="text-xs text-destructive">
                {errorFor("sql")}
              </p>
            ) : null}
          </div>

          <div className="flex items-center gap-2">
            <Switch id="cron-enabled" checked={enabled} onCheckedChange={setEnabled} />
            <Label htmlFor="cron-enabled">Enabled</Label>
          </div>

          <DialogFooter>
            {inline ? null : (
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
            )}
            <Button type="submit" disabled={save.isPending}>
              {save.isPending ? <Spinner className="size-3.5" /> : null}
              {job ? "Save" : "Create"}
            </Button>
          </DialogFooter>
        </form>
  );

  if (inline) return formEl;
  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">{formEl}</DialogContent>
    </Dialog>
  );
}
