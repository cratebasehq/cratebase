import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ListChecks, RotateCcw, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { cb, describeFailure, parseServerDate } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
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
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { SettingsPage } from "@/components/settings/settings-form";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

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

function statusBadgeClass(status: JobStatus): string {
  switch (status) {
    case "completed":
      return "font-normal text-success";
    case "failed":
      return "font-normal text-destructive";
    case "in_progress":
      return "font-normal text-info";
    default:
      return "font-normal text-muted-foreground";
  }
}

/**
 * Durable, retrying background jobs (`_queue_jobs`) — handled by a JS
 * `onQueueJob` handler in `pb_hooks/*.pb.js` or a Rust
 * `QueueHandle::register_handler`. This page is read/retry/delete only:
 * jobs are created by application code (`$queue.enqueue`/
 * `POST /api/queue/enqueue`), never by hand from here.
 */
export function QueueJobsPage() {
  const queryClient = useQueryClient();
  const [statusFilter, setStatusFilter] = useState<JobStatus | "all">("all");
  const [payloadJob, setPayloadJob] = useState<QueueJob | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<QueueJob | null>(null);

  const { data, isLoading, error } = useQuery({
    queryKey: ["queue-jobs", statusFilter],
    queryFn: () =>
      cb.collection("_queue_jobs").list({
        page: 1,
        perPage: PAGE_SIZE,
        sort: "-created",
        filter: statusFilter === "all" ? undefined : `status = "${statusFilter}"`,
      }) as unknown as Promise<{ items: QueueJob[]; totalItems: number }>,
  });

  const retry = useMutation({
    mutationFn: (id: string) => cb.send<void>(`/api/queue/jobs/${encodeURIComponent(id)}/retry`, { method: "POST" }),
    onSuccess: () => {
      toast.success("Job reset to pending");
      void queryClient.invalidateQueries({ queryKey: ["queue-jobs"] });
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.send<void>(`/api/queue/jobs/${encodeURIComponent(id)}`, { method: "DELETE" }),
    onSuccess: () => {
      toast.success("Job deleted");
      void queryClient.invalidateQueries({ queryKey: ["queue-jobs"] });
      setDeleteTarget(null);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const jobs = data?.items ?? [];

  // Per-queue counts, grouped from the fetched page — an aggregate across
  // every job ever enqueued would need a dedicated endpoint this doesn't
  // have yet, so with more than `PAGE_SIZE` jobs outstanding this is a
  // sample, not a total.
  const counts = new Map<string, Map<JobStatus, number>>();
  for (const job of jobs) {
    const byStatus = counts.get(job.queue) ?? new Map<JobStatus, number>();
    byStatus.set(job.status, (byStatus.get(job.status) ?? 0) + 1);
    counts.set(job.queue, byStatus);
  }

  const item = settingsItemFor("/settings/queue")!;
  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={
        <Select value={statusFilter} onValueChange={(v) => setStatusFilter(v as JobStatus | "all")}>
          <SelectTrigger className="w-40">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All statuses</SelectItem>
            {STATUSES.map((s) => (
              <SelectItem key={s} value={s}>
                {s}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      }
    >
      {counts.size > 0 ? (
        <div className="flex flex-wrap gap-2">
          {Array.from(counts.entries()).map(([queue, byStatus]) => (
            <Badge key={queue} variant="outline" className="font-normal">
              {queue}:{" "}
              {Array.from(byStatus.entries())
                .map(([status, n]) => `${n} ${status}`)
                .join(" · ")}
            </Badge>
          ))}
        </div>
      ) : null}

      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <ListChecks />
            </EmptyMedia>
            <EmptyTitle>Couldn't load queue jobs</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : isLoading ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 3 }, (_, i) => (
            <Skeleton key={i} className="h-row w-full" />
          ))}
        </div>
      ) : jobs.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <ListChecks />
            </EmptyMedia>
            <EmptyTitle>No jobs</EmptyTitle>
            <EmptyDescription>
              Nothing enqueued yet — call <code>$queue.enqueue(...)</code> from a hook, or{" "}
              <code>POST /api/queue/enqueue</code>.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Queue</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Attempts</TableHead>
              <TableHead>Runs at</TableHead>
              <TableHead className="w-32 text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {jobs.map((job) => (
              <TableRow key={job.id} className="align-top">
                <TableCell className="py-3">
                  <button
                    type="button"
                    className="font-medium underline-offset-2 hover:underline"
                    onClick={() => setPayloadJob(job)}
                  >
                    {job.queue}
                  </button>
                  {job.dedupeKey ? (
                    <div className="mt-1 font-mono text-[11px] text-muted-foreground/70">
                      dedupeKey: {job.dedupeKey}
                    </div>
                  ) : null}
                </TableCell>
                <TableCell className="py-3">
                  <Badge variant="outline" className={statusBadgeClass(job.status)}>
                    {job.status}
                  </Badge>
                  {job.status === "failed" && job.lastError ? (
                    <div className="mt-1 max-w-xs truncate text-xs text-muted-foreground" title={job.lastError}>
                      {job.lastError}
                    </div>
                  ) : null}
                </TableCell>
                <TableCell className="py-3 text-sm">
                  {job.attempts} / {job.maxAttempts}
                </TableCell>
                <TableCell className="py-3 text-xs text-muted-foreground">
                  {parseServerDate(job.runAfter).toLocaleString()}
                </TableCell>
                <TableCell className="py-3 text-right">
                  <div className="flex justify-end gap-1">
                    {job.status === "failed" ? (
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label={`Retry ${job.queue}`}
                        disabled={retry.isPending && retry.variables === job.id}
                        onClick={() => retry.mutate(job.id)}
                      >
                        {retry.isPending && retry.variables === job.id ? (
                          <Spinner className="size-3.5" />
                        ) : (
                          <RotateCcw className="size-3.5" />
                        )}
                      </Button>
                    ) : null}
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Delete ${job.queue} job`}
                      onClick={() => setDeleteTarget(job)}
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      <Dialog open={payloadJob !== null} onOpenChange={(open) => !open && setPayloadJob(null)}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{payloadJob?.queue}</DialogTitle>
            <DialogDescription>Payload and, if present, the most recent error.</DialogDescription>
          </DialogHeader>
          <pre className="max-h-80 overflow-auto rounded-md bg-muted p-3 text-xs">
            {JSON.stringify(payloadJob?.payload, null, 2)}
          </pre>
          {payloadJob?.lastError ? (
            <div className="flex flex-col gap-1.5">
              <p className="text-sm font-medium">Last error</p>
              <pre className="max-h-40 overflow-auto rounded-md bg-destructive/10 p-3 text-xs text-destructive">
                {payloadJob.lastError}
              </pre>
            </div>
          ) : null}
          <DialogFooter>
            <Button variant="outline" onClick={() => setPayloadJob(null)}>
              Close
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete this "{deleteTarget?.queue}" job?</DialogTitle>
            <DialogDescription>There is no undo.</DialogDescription>
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
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsPage>
  );
}
