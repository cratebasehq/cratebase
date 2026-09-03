import { useState } from "react";
import { toast } from "sonner";
import { Pencil, Plus, Trash2 } from "lucide-react";
import { useRecords, useRecordMutations } from "@/hooks/use-records";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { CronJobDrawer, type CronJobRecord } from "@/components/settings/cron-job-drawer";

const COLLECTION = "_cron_jobs";

/** Friendlier front end for the `_cron_jobs` collection the cron-jobs
 * server plugin already reads from — reuses `useRecords`/`useRecordMutations`
 * (the same data layer the generic collection records UI runs on) instead
 * of a parallel fetch path, so this is purely a nicer editor over the same
 * rows `/collections/_cron_jobs` would show. */
export function CronJobsPage() {
  const { data } = useRecords(COLLECTION, 1, "", "name");
  const { update, remove } = useRecordMutations(COLLECTION);
  const [editing, setEditing] = useState<CronJobRecord | null | undefined>(undefined);

  const jobs = (data?.items ?? []) as CronJobRecord[];

  function toggleEnabled(job: CronJobRecord) {
    update
      .mutateAsync({ id: job.id, data: { enabled: !job.enabled } })
      .catch((error) => toast.error(error instanceof Error ? error.message : "Failed to update job"));
  }

  function deleteJob(job: CronJobRecord) {
    remove
      .mutateAsync(job.id)
      .then(() => toast.success(`Cron job "${job.name}" deleted`))
      .catch((error) => toast.error(error instanceof Error ? error.message : "Failed to delete job"));
  }

  return (
    <div className="flex flex-col gap-4 p-6">
      <div className="flex items-center justify-between">
        <p className="text-[12.5px] text-muted-foreground">
          Scheduled jobs the server checks every 30 seconds against their cron expression.
        </p>
        <Button onClick={() => setEditing(null)}>
          <Plus className="size-3.5" />
          New job
        </Button>
      </div>

      <Table>
        <TableHeader>
          <TableRow>
            <TableHead className="w-[70px]">Enabled</TableHead>
            <TableHead>Name</TableHead>
            <TableHead className="w-[140px]">Schedule</TableHead>
            <TableHead>Job</TableHead>
            <TableHead className="w-[160px]">Last run</TableHead>
            <TableHead className="w-[100px]" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {jobs.map((job) => (
            <TableRow key={job.id}>
              <TableCell>
                <Switch checked={job.enabled} onCheckedChange={() => toggleEnabled(job)} aria-label={`Toggle ${job.name}`} />
              </TableCell>
              <TableCell className="font-medium">{job.name}</TableCell>
              <TableCell className="font-mono text-xs">{job.schedule}</TableCell>
              <TableCell className="font-mono text-xs text-muted-foreground">{job.job}</TableCell>
              <TableCell className="text-xs text-muted-foreground">
                {job.lastRunAt ? (
                  <span className="flex items-center gap-1.5">
                    {new Date(job.lastRunAt).toLocaleString()}
                    {job.lastStatus ? (
                      <Badge variant={job.lastStatus === "ok" ? "default" : "destructive"}>{job.lastStatus}</Badge>
                    ) : null}
                  </span>
                ) : (
                  "never"
                )}
              </TableCell>
              <TableCell>
                <div className="flex justify-end gap-1">
                  <Button variant="ghost" size="icon-sm" aria-label={`Edit ${job.name}`} onClick={() => setEditing(job)}>
                    <Pencil className="size-3.5" />
                  </Button>
                  <Button variant="ghost" size="icon-sm" aria-label={`Delete ${job.name}`} onClick={() => deleteJob(job)}>
                    <Trash2 className="size-3.5 text-destructive" />
                  </Button>
                </div>
              </TableCell>
            </TableRow>
          ))}
          {jobs.length === 0 ? (
            <TableRow>
              <TableCell colSpan={6} className="py-8 text-center text-sm text-muted-foreground">
                No cron jobs configured.
              </TableCell>
            </TableRow>
          ) : null}
        </TableBody>
      </Table>

      <CronJobDrawer job={editing ?? null} open={editing !== undefined} onOpenChange={(open) => !open && setEditing(undefined)} />
    </div>
  );
}
