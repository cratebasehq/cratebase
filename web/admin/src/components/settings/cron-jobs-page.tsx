import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Clock, Pencil, Play, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { cb } from "@/lib/api";
import { describeFailure } from "@/lib/api";
import { describeJob, describeSchedule } from "@/lib/cron";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";

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

/**
 * Jobs the server has registered, and a way to run one now.
 *
 * Two halves: the four-and-a-bit **system** jobs (log trimming, expired
 * codes, database upkeep, the scheduled backup) are registered in code —
 * there is no record backing them, so that table only lists and triggers.
 * **Custom** jobs are ordinary `_cron_jobs` records: a name, a cron
 * expression, and a raw SQL statement to run on it — full CRUD, because
 * unlike the system jobs an operator is expected to create, edit and
 * remove these. Both halves share the same `POST /api/crons/{id}` "run
 * now" endpoint; a custom job's id there is `custom:<record id>`.
 */
export function CronJobsPage() {
  const queryClient = useQueryClient();

  const { data, isLoading, error } = useQuery({
    queryKey: ["crons"],
    queryFn: () => cb.send<CronJob[]>("/api/crons", { method: "GET" }),
  });

  const run = useMutation({
    mutationFn: (id: string) => cb.send<void>(`/api/crons/${encodeURIComponent(id)}`, { method: "POST" }),
    onSuccess: (_result, id) => {
      toast.success(`Ran ${describeJob(id).title}`);
      // A job that touches the log or the database changes what the other
      // settings screens show.
      void queryClient.invalidateQueries({ queryKey: ["request-logs"] });
      void queryClient.invalidateQueries({ queryKey: ["backups"] });
      void queryClient.invalidateQueries({ queryKey: ["custom-cron-jobs"] });
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const jobs = (data ?? []).filter((job) => !job.id.startsWith("custom:"));

  return (
    <div className="flex flex-col gap-page p-page">
      <div className="flex flex-col gap-2">
        <h2 className="text-sm font-medium">System jobs</h2>
        <p className="max-w-measure text-sm text-muted-foreground">
          Scheduled work the server runs on its own — log trimming, expired one-time codes, database upkeep, and the
          automatic backup once one is configured. Running a job here executes it immediately without affecting its
          schedule.
        </p>
      </div>

      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Clock />
            </EmptyMedia>
            <EmptyTitle>Couldn't load the schedule</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : isLoading ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 4 }, (_, i) => (
            <Skeleton key={i} className="h-row w-full" />
          ))}
        </div>
      ) : jobs.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Clock />
            </EmptyMedia>
            <EmptyTitle>No scheduled jobs</EmptyTitle>
            <EmptyDescription>Nothing is registered on this server.</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table className="max-w-4xl">
          <TableHeader>
            <TableRow>
              <TableHead>Job</TableHead>
              <TableHead>Runs</TableHead>
              <TableHead className="w-24 text-right">Run</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {jobs.map((job) => {
              const described = describeJob(job.id);
              const schedule = describeSchedule(job.expression);
              const running = run.isPending && run.variables === job.id;
              return (
                <TableRow key={job.id} className="align-top">
                  <TableCell className="py-3">
                    <div className="font-medium">{described.title}</div>
                    <div className="text-xs text-muted-foreground">{described.detail}</div>
                    {/* The id is what `POST /api/crons/{id}` takes, so it stays
                        visible for anyone scripting against the API — just not
                        as the thing you read first. */}
                    <div className="mt-1 font-mono text-[11px] text-muted-foreground/70">{job.id}</div>
                  </TableCell>
                  <TableCell className="py-3 text-sm">
                    {schedule}
                    {schedule === job.expression ? null : (
                      <div className="font-mono text-[11px] text-muted-foreground/70">{job.expression}</div>
                    )}
                  </TableCell>
                  <TableCell className="py-3 text-right">
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Run ${described.title} now`}
                      disabled={running}
                      onClick={() => run.mutate(job.id)}
                    >
                      {running ? <Spinner className="size-3.5" /> : <Play className="size-3.5" />}
                      Run now
                    </Button>
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      )}

      <CustomCronJobsSection onRun={(id) => run.mutate(id)} runningId={run.isPending ? (run.variables ?? null) : null} />
    </div>
  );
}

/** `sql = "invalid"` or an empty `expression` produces a server-side field
 * error (`crates/server/src/cron_jobs.rs`'s `on_record_create`/
 * `on_record_update` hooks reject it before the row is even written), so
 * the create/edit dialog below surfaces that instead of a generic toast —
 * the same `fields` pattern the login form uses for its own errors. */
function CustomCronJobsSection({
  onRun,
  runningId,
}: {
  onRun: (id: string) => void;
  runningId: string | null;
}) {
  const queryClient = useQueryClient();
  const [dialogJob, setDialogJob] = useState<CustomCronJob | "new" | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<CustomCronJob | null>(null);

  const { data, isLoading, error } = useQuery({
    queryKey: ["custom-cron-jobs"],
    queryFn: () =>
      cb.collection("_cron_jobs").getFullList<CustomCronJob>({ sort: "-created", requestKey: null }),
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection("_cron_jobs").delete(id),
    onSuccess: () => {
      toast.success("Custom job deleted");
      void queryClient.invalidateQueries({ queryKey: ["custom-cron-jobs"] });
      void queryClient.invalidateQueries({ queryKey: ["crons"] });
      setDeleteTarget(null);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const jobs = data ?? [];

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="flex flex-col gap-2">
          <h2 className="text-sm font-medium">Custom jobs</h2>
          <p className="max-w-measure text-sm text-muted-foreground">
            Your own scheduled SQL. Runs with no rule enforcement — the same trust tier as editing a collection's
            schema — so this is a superuser tool, not something to script from application code.
          </p>
        </div>
        <Button size="sm" className="gap-1.5 shrink-0" onClick={() => setDialogJob("new")}>
          <Plus className="size-3.5" />
          New job
        </Button>
      </div>

      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Clock />
            </EmptyMedia>
            <EmptyTitle>Couldn't load custom jobs</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : isLoading ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 2 }, (_, i) => (
            <Skeleton key={i} className="h-row w-full" />
          ))}
        </div>
      ) : jobs.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Clock />
            </EmptyMedia>
            <EmptyTitle>No custom jobs yet</EmptyTitle>
            <EmptyDescription>Add one to run a SQL statement on a schedule.</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table className="max-w-4xl">
          <TableHeader>
            <TableRow>
              <TableHead>Job</TableHead>
              <TableHead>Runs</TableHead>
              <TableHead>Last run</TableHead>
              <TableHead className="w-32 text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {jobs.map((job) => {
              const jobId = `custom:${job.id}`;
              const running = runningId === jobId;
              const schedule = describeSchedule(job.expression);
              return (
                <TableRow key={job.id} className="align-top">
                  <TableCell className="py-3">
                    <div className="flex items-center gap-2 font-medium">
                      {job.name}
                      {!job.enabled ? (
                        <Badge variant="outline" className="font-normal text-muted-foreground">
                          Disabled
                        </Badge>
                      ) : null}
                    </div>
                    <div className="mt-1 max-w-sm truncate font-mono text-[11px] text-muted-foreground/70">
                      {job.sql}
                    </div>
                  </TableCell>
                  <TableCell className="py-3 text-sm">
                    {schedule}
                    {schedule === job.expression ? null : (
                      <div className="font-mono text-[11px] text-muted-foreground/70">{job.expression}</div>
                    )}
                  </TableCell>
                  <TableCell className="py-3 text-xs text-muted-foreground">
                    {job.lastRunAt ? (
                      <>
                        <Badge
                          variant="outline"
                          className={
                            job.lastStatus === "error"
                              ? "font-normal text-destructive"
                              : "font-normal text-success"
                          }
                        >
                          {job.lastStatus ?? "unknown"}
                        </Badge>
                        <div className="mt-1">{new Date(job.lastRunAt).toLocaleString()}</div>
                        {job.lastMessage ? <div className="truncate">{job.lastMessage}</div> : null}
                      </>
                    ) : (
                      "Never run"
                    )}
                  </TableCell>
                  <TableCell className="py-3 text-right">
                    <div className="flex justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label={`Run ${job.name} now`}
                        disabled={running}
                        onClick={() => onRun(jobId)}
                      >
                        {running ? <Spinner className="size-3.5" /> : <Play className="size-3.5" />}
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label={`Edit ${job.name}`}
                        onClick={() => setDialogJob(job)}
                      >
                        <Pencil className="size-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label={`Delete ${job.name}`}
                        onClick={() => setDeleteTarget(job)}
                      >
                        <Trash2 className="size-3.5" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      )}

      {dialogJob ? (
        <CustomCronJobDialog
          job={dialogJob === "new" ? null : dialogJob}
          onOpenChange={(open) => {
            if (!open) setDialogJob(null);
          }}
        />
      ) : null}

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete "{deleteTarget?.name}"?</DialogTitle>
            <DialogDescription>
              This stops the schedule immediately. There is no undo.
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
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** Create or edit one `_cron_jobs` record. `job === null` means create. */
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
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  const save = useMutation({
    mutationFn: () => {
      const body = { name, expression, sql, enabled };
      return job ? cb.collection("_cron_jobs").update(job.id, body) : cb.collection("_cron_jobs").create(body);
    },
    onSuccess: () => {
      toast.success(job ? "Custom job updated" : "Custom job created");
      void queryClient.invalidateQueries({ queryKey: ["custom-cron-jobs"] });
      void queryClient.invalidateQueries({ queryKey: ["crons"] });
      onOpenChange(false);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      setFieldErrors(described.fields);
      if (Object.keys(described.fields).length === 0) {
        toast.error(described.title, { description: described.detail });
      }
    },
  });

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{job ? "Edit custom job" : "New custom job"}</DialogTitle>
          <DialogDescription>
            Runs with no rule enforcement — treat the SQL below the same as a collection schema edit.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="cron-name">Name</Label>
            <Input id="cron-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Nightly rollup" />
            {fieldErrors["name"] ? <p className="text-xs text-destructive">{fieldErrors["name"]}</p> : null}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="cron-expression">Cron expression</Label>
            <Input
              id="cron-expression"
              className="font-mono"
              value={expression}
              onChange={(e) => setExpression(e.target.value)}
              placeholder="0 0 * * *"
            />
            <p className="text-xs text-muted-foreground">{describeSchedule(expression)}</p>
            {fieldErrors["expression"] ? (
              <p className="text-xs text-destructive">{fieldErrors["expression"]}</p>
            ) : null}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="cron-sql">SQL</Label>
            <Textarea
              id="cron-sql"
              className="min-h-24 font-mono text-xs"
              value={sql}
              onChange={(e) => setSql(e.target.value)}
              placeholder='DELETE FROM "posts" WHERE "created" < date(&apos;now&apos;, &apos;-30 days&apos;)'
            />
            {fieldErrors["sql"] ? <p className="text-xs text-destructive">{fieldErrors["sql"]}</p> : null}
          </div>

          <div className="flex items-center gap-2">
            <Switch id="cron-enabled" checked={enabled} onCheckedChange={setEnabled} />
            <Label htmlFor="cron-enabled">Enabled</Label>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={save.isPending} onClick={() => save.mutate()}>
            {save.isPending ? <Spinner className="size-3.5" /> : null}
            {job ? "Save" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
