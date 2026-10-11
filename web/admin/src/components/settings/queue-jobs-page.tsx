import { useState } from "react";
import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { CircleAlert, Check, Eye, ListChecks, LoaderCircle, RotateCcw, Timer, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { Link } from "@tanstack/react-router";
import { cb, describeFailure } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
import { useSettings } from "@/hooks/use-settings";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { StatCard, StatusBadge, type Tone } from "@/components/layout/stat-card";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { SettingsPage } from "@/components/settings/settings-form";
import { LIVE_INTERVAL_MS, LiveToggle } from "@/components/settings/live-toggle";
import { TimeAgo } from "@/components/settings/time-ago";
import {
  DetailBody,
  DetailCallout,
  DetailList,
  DetailRow,
  DetailSection,
  DetailSheet,
  DetailSheetHeader,
} from "@/components/layout/detail-sheet";
import { CodeBlock } from "@/components/settings/detail-json";

const STATUSES = ["pending", "in_progress", "completed", "failed"] as const;
type JobStatus = (typeof STATUSES)[number];

/** A `_queue_jobs` record — see `crates/server/src/queue.rs`. */
interface QueueJob {
  id: string;
  queue: string;
  payload: unknown;
  status: JobStatus;
  attempts: number;
  maxAttempts: number;
  runAfter: string;
  lastError?: string;
  dedupeKey?: string;
  priority: number;
  created: string;
}

const PAGE_SIZE = 50;

/** What each stored status is called on screen. */
const STATUS_LABEL: Record<JobStatus, string> = {
  pending: "Pending",
  in_progress: "Running",
  completed: "Done",
  failed: "Failed",
};

const STATUS_TONE: Record<JobStatus, Tone> = {
  pending: "neutral",
  in_progress: "info",
  completed: "success",
  failed: "danger",
};

/**
 * Durable, retrying background jobs (`_queue_jobs`) — handled by a JS
 * `onQueueJob` handler in `cb_hooks/*.cb.js` or a Rust
 * `QueueHandle::register_handler`. This page is read/retry/delete only:
 * jobs are created by application code (`$queue.enqueue`/
 * `POST /api/queue/enqueue`), never by hand from here. It refreshes itself
 * every few seconds while the tab is visible and no dialog is open.
 */
export function QueueJobsPage() {
  const queryClient = useQueryClient();
  const [statusFilter, setStatusFilter] = useState<JobStatus | "all">("all");
  const [search, setSearch] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<QueueJob | null>(null);

  const [live, setLive] = useState(true);
  // Hold still while a dialog is open so rows don't shift under the operator.
  const refetchInterval = live && selectedId === null && deleteTarget === null ? LIVE_INTERVAL_MS : false;

  // The queue is a toggle-gated module: while it is off its collection does not
  // exist, so don't query it at all.
  const settings = useSettings();
  const queueOff = settings.data?.queue?.enabled === false;
  const queryEnabled = !settings.isPending && !queueOff;

  const jobsQuery = useQuery({
    enabled: queryEnabled,
    queryKey: ["queue-jobs", statusFilter],
    queryFn: () =>
      cb.collection("_queue_jobs").list({
        page: 1,
        perPage: PAGE_SIZE,
        sort: "-created",
        filter: statusFilter === "all" ? undefined : `status = "${statusFilter}"`,
      }) as unknown as Promise<{ items: QueueJob[]; totalItems: number }>,
    refetchInterval,
  });

  // Exact totals per status: a one-row page whose `totalItems` is the count.
  const countQueries = useQueries({
    queries: STATUSES.map((status) => ({
      enabled: queryEnabled,
      queryKey: ["queue-jobs-count", status],
      queryFn: () =>
        cb.collection("_queue_jobs").list({
          page: 1,
          perPage: 1,
          filter: `status = "${status}"`,
        }) as unknown as Promise<{ totalItems: number }>,
      refetchInterval,
    })),
  });
  const countOf = (status: JobStatus) => countQueries[STATUSES.indexOf(status)]?.data?.totalItems;
  const countsLoading = countQueries.some((q) => q.isPending);

  function refreshAll() {
    void queryClient.invalidateQueries({ queryKey: ["queue-jobs"] });
    void queryClient.invalidateQueries({ queryKey: ["queue-jobs-count"] });
  }

  const retry = useMutation({
    mutationFn: (id: string) => cb.send<void>(`/api/queue/jobs/${encodeURIComponent(id)}/retry`, { method: "POST" }),
    onSuccess: () => {
      toast.success("Job reset to pending");
      refreshAll();
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error("Couldn't retry the job", { description: described.detail });
    },
  });

  const retryAll = useMutation({
    mutationFn: async (ids: string[]) => {
      const results = await Promise.allSettled(
        ids.map((id) => cb.send<void>(`/api/queue/jobs/${encodeURIComponent(id)}/retry`, { method: "POST" })),
      );
      const failed = results.filter((r) => r.status === "rejected");
      if (failed.length === results.length) throw (failed[0] as PromiseRejectedResult).reason;
      return { ok: results.length - failed.length, failed: failed.length };
    },
    onSuccess: ({ ok, failed }) => {
      if (failed === 0) toast.success(`Reset ${ok} failed ${ok === 1 ? "job" : "jobs"} to pending`);
      else toast.warning(`Reset ${ok} jobs; ${failed} couldn't be retried`);
      refreshAll();
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error("Couldn't retry the failed jobs", { description: described.detail });
      refreshAll();
    },
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.send<void>(`/api/queue/jobs/${encodeURIComponent(id)}`, { method: "DELETE" }),
    onSuccess: () => {
      toast.success("Job deleted");
      refreshAll();
      setDeleteTarget(null);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error("Couldn't delete the job", { description: described.detail });
    },
  });

  const needle = search.trim().toLowerCase();
  const jobs = (jobsQuery.data?.items ?? []).filter(
    (job) =>
      !needle ||
      job.queue.toLowerCase().includes(needle) ||
      job.id.toLowerCase().includes(needle) ||
      (job.dedupeKey ?? "").toLowerCase().includes(needle),
  );
  const failedJobs = jobs.filter((job) => job.status === "failed");
  const total = jobsQuery.data?.totalItems ?? 0;
  const failedCount = countOf("failed") ?? 0;
  const filtering = statusFilter !== "all" || needle !== "";

  const item = settingsItemFor("/settings/queue")!;
  const selectedJob = selectedId ? jobs.find((job) => job.id === selectedId) : undefined;
  if (queueOff) {
    return (
      <SettingsPage title={item.label} description={item.description} width="wide">
        <EmptyState
          compact
          icon={<ListChecks />}
          title="The Queue module is turned off"
          description="Background jobs with retries are an optional module. Turn it on under Application → Modules, then enqueue jobs from a hook or the API."
          action={
            <Button size="sm" asChild>
              <Link to="/settings/application" search={{ tab: "modules" }}>
                Open Modules
              </Link>
            </Button>
          }
        />
      </SettingsPage>
    );
  }
  return (
    <SettingsPage title={item.label} description={item.description} width="wide">
      <div className="grid grid-cols-2 gap-2 lg:grid-cols-4">
        <StatCard
          index={0}
          label="Pending"
          icon={<Timer />}
          loading={countsLoading}
          value={countOf("pending") ?? 0}
          hint="Waiting for a worker"
        />
        <StatCard
          index={1}
          label="Running"
          icon={<LoaderCircle />}
          loading={countsLoading}
          value={countOf("in_progress") ?? 0}
          hint="Being handled now"
        />
        <StatCard
          index={2}
          label="Failed"
          icon={<CircleAlert />}
          loading={countsLoading}
          value={failedCount}
          tone={failedCount > 0 ? "danger" : "neutral"}
          hint={failedCount > 0 ? "Out of attempts — retry or delete" : "Nothing needs attention"}
        />
        <StatCard
          index={3}
          label="Done"
          icon={<Check />}
          loading={countsLoading}
          value={countOf("completed") ?? 0}
          hint="Completed successfully"
        />
      </div>

      <DataTableToolbar
        search={{ value: search, onChange: setSearch, placeholder: "Filter by queue, id or key…", label: "Filter jobs" }}
        filters={
          <Select value={statusFilter} onValueChange={(v) => setStatusFilter(v as JobStatus | "all")}>
            <SelectTrigger size="sm" aria-label="Filter by status" className="w-36">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All statuses</SelectItem>
              {STATUSES.map((s) => (
                <SelectItem key={s} value={s}>
                  {STATUS_LABEL[s]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        }
        actions={
          <>
            <LiveToggle live={live} onLiveChange={setLive} fetching={jobsQuery.isFetching} onRefresh={refreshAll} />
            {failedJobs.length > 0 ? (
              <Button
                variant="outline"
                size="sm"
                disabled={retryAll.isPending}
                onClick={() => retryAll.mutate(failedJobs.map((job) => job.id))}
              >
                {retryAll.isPending ? <Spinner /> : <RotateCcw />}
                Retry {failedJobs.length} failed
              </Button>
            ) : null}
          </>
        }
      />

      {jobsQuery.isError ? (
        <ErrorState
          title="Couldn't load queue jobs"
          error={jobsQuery.error}
          onRetry={() => void jobsQuery.refetch()}
          compact
        />
      ) : jobsQuery.isPending ? (
        <ListSkeleton rows={6} />
      ) : jobs.length === 0 ? (
        filtering ? (
          <EmptyState
            compact
            icon={<ListChecks />}
            title={needle ? "No jobs match" : `No ${STATUS_LABEL[statusFilter as JobStatus].toLowerCase()} jobs`}
            description="Nothing in the newest jobs fits this filter."
            action={
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  setSearch("");
                  setStatusFilter("all");
                }}
              >
                Clear filters
              </Button>
            }
          />
        ) : (
          <EmptyState
            compact
            icon={<ListChecks />}
            title="The queue is empty"
            description={
              <>
                Jobs are created by your code, never by hand. Call{" "}
                <code className="font-mono">$queue.enqueue(name, payload)</code> from a{" "}
                <code className="font-mono">cb_hooks/*.cb.js</code> hook, or{" "}
                <code className="font-mono">POST /api/queue/enqueue</code>. Handle them with{" "}
                <code className="font-mono">onQueueJob</code>.
              </>
            }
          />
        )
      ) : (
        <div className="rounded-lg border border-border bg-card overflow-hidden">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Queue</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Attempts</TableHead>
                <TableHead>Runs at</TableHead>
                <TableHead>Enqueued</TableHead>
                <TableHead className="w-28 text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {jobs.map((job) => {
                const retrying = retry.isPending && retry.variables === job.id;
                return (
                  <TableRow key={job.id} className="align-top">
                    <TableCell className="py-2.5">
                      <button
                        type="button"
                        className="rounded-sm font-mono text-sm font-medium underline-offset-2 hover:underline"
                        onClick={() => setSelectedId(job.id)}
                      >
                        {job.queue}
                      </button>
                      <div className="font-mono text-xs text-muted-foreground">{job.id}</div>
                      {job.dedupeKey ? (
                        <div className="font-mono text-xs text-muted-foreground">dedupeKey: {job.dedupeKey}</div>
                      ) : null}
                    </TableCell>
                    <TableCell className="py-2.5">
                      <StatusBadge tone={STATUS_TONE[job.status]}>{STATUS_LABEL[job.status]}</StatusBadge>
                      {job.status === "failed" && job.lastError ? (
                        <div
                          className="mt-1 max-w-xs truncate font-mono text-xs text-destructive"
                          title={job.lastError}
                        >
                          {job.lastError}
                        </div>
                      ) : null}
                    </TableCell>
                    <TableCell className="font-tabular py-2.5 font-mono text-sm">
                      {job.attempts} / {job.maxAttempts}
                    </TableCell>
                    <TableCell className="py-2.5 text-xs">
                      <TimeAgo value={job.runAfter} />
                    </TableCell>
                    <TableCell className="py-2.5 text-xs text-muted-foreground">
                      <TimeAgo value={job.created} />
                    </TableCell>
                    <TableCell className="py-2.5 text-right">
                      <div className="flex justify-end gap-0.5">
                        <Button
                          variant="ghost"
                          size="icon-xs"
                          aria-label={`View payload of ${job.queue} job ${job.id}`}
                          onClick={() => setSelectedId(job.id)}
                        >
                          <Eye />
                        </Button>
                        {job.status === "failed" ? (
                          <Button
                            variant="ghost"
                            size="icon-xs"
                            aria-label={`Retry ${job.queue} job ${job.id}`}
                            disabled={retrying}
                            onClick={() => retry.mutate(job.id)}
                          >
                            {retrying ? <Spinner className="size-3" /> : <RotateCcw />}
                          </Button>
                        ) : null}
                        <Button
                          variant="ghost"
                          size="icon-xs"
                          aria-label={`Delete ${job.queue} job ${job.id}`}
                          onClick={() => setDeleteTarget(job)}
                        >
                          <Trash2 />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      )}

      {jobsQuery.data && total > PAGE_SIZE ? (
        <p className="font-tabular text-xs text-muted-foreground">
          Showing the newest {PAGE_SIZE} of {total} jobs{statusFilter === "all" ? "" : ` (${STATUS_LABEL[statusFilter].toLowerCase()})`}.
        </p>
      ) : null}

      {selectedJob ? (
        <QueueJobSheet
          job={selectedJob}
          jobs={jobs}
          onSelect={setSelectedId}
          onClose={() => setSelectedId(null)}
          onRetry={(id) => retry.mutate(id)}
          retrying={retry.isPending && retry.variables === selectedJob.id}
          onDelete={setDeleteTarget}
        />
      ) : null}

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete this "{deleteTarget?.queue}" job?</DialogTitle>
            <DialogDescription>
              Job <code className="font-mono">{deleteTarget?.id}</code>
              {deleteTarget?.status === "in_progress" ? " is running right now; deleting it won't stop the handler." : " will be removed from the queue."}{" "}
              There is no undo.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={remove.isPending}
              onClick={() => deleteTarget && remove.mutate(deleteTarget.id)}
            >
              {remove.isPending ? <Spinner className="size-3.5" /> : null}
              Delete job
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsPage>
  );
}

/** Detail drawer for one `_queue_jobs` record: state, attempts, the payload and the last error. */
function QueueJobSheet({
  job,
  jobs,
  onSelect,
  onClose,
  onRetry,
  retrying,
  onDelete,
}: {
  job: QueueJob;
  jobs: QueueJob[];
  onSelect: (id: string) => void;
  onClose: () => void;
  onRetry: (id: string) => void;
  retrying: boolean;
  onDelete: (job: QueueJob) => void;
}) {
  const payload = JSON.stringify(job.payload, null, 2) ?? "null";
  const index = jobs.findIndex((j) => j.id === job.id);
  const pager =
    index >= 0
      ? {
          index: index + 1,
          total: jobs.length,
          hasPrev: index > 0,
          hasNext: index < jobs.length - 1,
          onPrev: () => onSelect(jobs[index - 1].id),
          onNext: () => onSelect(jobs[index + 1].id),
        }
      : undefined;
  const attemptsLeft = Math.max(0, job.maxAttempts - job.attempts);

  return (
    <DetailSheet open onOpenChange={(open) => !open && onClose()}>
      <DetailSheetHeader
        id={job.id}
        pager={pager}
        title={<span className="font-mono">{job.queue}</span>}
        description={`${STATUS_LABEL[job.status]} queue job ${job.id}`}
        status={<StatusBadge tone={STATUS_TONE[job.status]}>{STATUS_LABEL[job.status]}</StatusBadge>}
        meta={[
          <span key="attempts" className="font-tabular">
            {job.attempts}/{job.maxAttempts} attempts
          </span>,
          <span key="priority" className="font-tabular">
            priority {job.priority}
          </span>,
        ]}
        actions={
          job.status === "failed" ? (
            <Button size="sm" disabled={retrying} onClick={() => onRetry(job.id)}>
              {retrying ? <Spinner className="size-3.5" /> : <RotateCcw />}
              Retry
            </Button>
          ) : undefined
        }
        menu={[{ label: "Delete job", icon: <Trash2 />, destructive: true, onSelect: () => onDelete(job) }]}
      />
      <DetailBody>
        {job.lastError ? (
          <DetailCallout
            tone={job.status === "failed" ? "danger" : "warning"}
            title={
              job.status === "failed"
                ? `Failed after ${job.attempts} ${job.attempts === 1 ? "attempt" : "attempts"}`
                : "Last attempt failed"
            }
          >
            <span className="font-mono break-words whitespace-pre-wrap">{job.lastError}</span>
          </DetailCallout>
        ) : null}
        <DetailSection title="Details">
          <DetailList>
            <DetailRow label="Status">
              <StatusBadge tone={STATUS_TONE[job.status]}>{STATUS_LABEL[job.status]}</StatusBadge>
            </DetailRow>
            <DetailRow label="Attempts" hint={`${attemptsLeft} left`}>
              <span className="font-tabular font-mono">
                {job.attempts} / {job.maxAttempts}
              </span>
            </DetailRow>
            <DetailRow label="Priority">
              <span className="font-tabular font-mono">{job.priority}</span>
            </DetailRow>
            <DetailRow label="Runs at">
              <TimeAgo value={job.runAfter} />
            </DetailRow>
            <DetailRow label="Enqueued">
              <TimeAgo value={job.created} />
            </DetailRow>
            {job.dedupeKey ? (
              <DetailRow label="Dedupe key">
                <span className="font-mono text-xs break-all">{job.dedupeKey}</span>
              </DetailRow>
            ) : null}
          </DetailList>
        </DetailSection>
        <DetailSection title="Payload">
          <CodeBlock json={payload} copyLabel="Copy payload" />
        </DetailSection>
      </DetailBody>
    </DetailSheet>
  );
}
