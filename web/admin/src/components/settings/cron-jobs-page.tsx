import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CalendarClock, CircleAlert, Clock, History, Pencil, Play, Plus, Trash2 } from "lucide-react";
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
  const [historyJob, setHistoryJob] = useState<{ id: string; title: string } | null>(null);

  const dialogOpen = dialogJob !== null || deleteTarget !== null || historyJob !== null;
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
    onSuccess: () => {
      toast.success(`Deleted ${deleteTarget?.name ?? "job"}`);
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
                            <span className="font-medium">{row.title}</span>
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
                              onClick={() => setHistoryJob({ id: row.id, title: row.title })}
                            >
                              <History />
                            </Button>
                            {customJob ? (
                              <>
                                <Button
                                  variant="ghost"
                                  size="icon-xs"
                                  aria-label={`Edit ${customJob.name}`}
                                  onClick={() => setDialogJob(customJob)}
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
        <CustomCronJobDialog
          job={dialogJob === "new" ? null : dialogJob}
          onOpenChange={(open) => {
            if (!open) setDialogJob(null);
          }}
        />
      ) : null}

      {historyJob ? (
        <CronHistoryDialog job={historyJob} onOpenChange={(open) => !open && setHistoryJob(null)} />
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

/** Run history for one job id — `GET /api/crons` gives `custom:<id>` for
 * a SQL `_cron_jobs` row or the raw `cronAdd` id otherwise, which is
 * exactly `_cronRuns.jobId`. Empty is normal: a job that's never fired,
 * or whose ticks all lost the multi-node advisory-lock race on this
 * node, has zero rows. */
function CronHistoryDialog({
  job,
  onOpenChange,
}: {
  job: { id: string; title: string };
  onOpenChange: (open: boolean) => void;
}) {
  const { data, isPending, error, refetch } = useQuery({
    queryKey: ["cron-runs", job.id],
    queryFn: () =>
      cb.collection("_cronRuns").list({
        page: 1,
        perPage: 20,
        sort: "-startedAt",
        filter: `jobId = "${job.id}"`,
      }) as unknown as Promise<{ items: CronRun[] }>,
  });

  const runs = data?.items ?? [];

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{job.title} — history</DialogTitle>
          <DialogDescription>The 20 most recent runs recorded in _cronRuns.</DialogDescription>
        </DialogHeader>

        {error ? (
          <ErrorState title="Couldn't load history" error={error} onRetry={() => void refetch()} compact />
        ) : isPending ? (
          <ListSkeleton rows={3} />
        ) : runs.length === 0 ? (
          <EmptyState
            compact
            icon={<History />}
            title="No runs recorded yet"
            description="A job that hasn't fired yet — or whose ticks ran on another node — has no history. Use Run now to record one."
          />
        ) : (
          <div className="max-h-96 overflow-auto rounded-lg border border-border">
            <ul className="divide-y divide-border">
              {runs.map((run) => (
                <li key={run.id} className="flex flex-col gap-1 px-3 py-2 text-xs">
                  <div className="flex flex-wrap items-center gap-2">
                    <StatusBadge tone={run.status === "success" ? "success" : "danger"}>
                      {run.status === "success" ? "Succeeded" : "Failed"}
                    </StatusBadge>
                    <Badge variant="outline" className="h-4 px-1.5 text-2xs font-normal uppercase">
                      {run.source}
                    </Badge>
                    <TimeAgo value={run.startedAt} className="text-muted-foreground" />
                    <span className="font-mono text-muted-foreground">
                      {parseServerDate(run.startedAt).toLocaleString()}
                    </span>
                    {typeof run.durationMs === "number" ? (
                      <span className="font-tabular ml-auto font-mono text-muted-foreground">{run.durationMs}ms</span>
                    ) : null}
                  </div>
                  {run.message ? (
                    <div
                      className={
                        run.status === "error"
                          ? "font-mono break-words text-destructive"
                          : "font-mono break-words text-muted-foreground"
                      }
                    >
                      {run.message}
                    </div>
                  ) : null}
                </li>
              ))}
            </ul>
          </div>
        )}

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** `sql = "invalid"` or an empty `expression` produces a server-side field
 * error (`crates/server/src/cron_jobs.rs`'s `on_record_create`/
 * `on_record_update` hooks reject it before the row is even written), so
 * this dialog surfaces that inline instead of a generic toast — alongside
 * the same checks done up front for an empty name, expression or SQL.
 * Create or edit one `_cron_jobs` record. `job === null` means create. */
function CustomCronJobDialog({
  job,
  onOpenChange,
}: {
  job: CustomCronJob | null;
  onOpenChange: (open: boolean) => void;
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

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
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
          <DialogHeader>
            <DialogTitle>{job ? "Edit SQL job" : "New SQL job"}</DialogTitle>
            <DialogDescription>
              Runs with no rule enforcement — treat the SQL below the same as a collection schema edit.
            </DialogDescription>
          </DialogHeader>

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
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {save.isPending ? <Spinner className="size-3.5" /> : null}
              {job ? "Save" : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
