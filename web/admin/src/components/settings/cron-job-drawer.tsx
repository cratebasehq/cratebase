import { useEffect, useState } from "react";
import { toast } from "sonner";
import type { RecordModel } from "cratebase";
import { Drawer } from "@/components/interior/drawer";
import { LoadingButton } from "@/components/interior/loading-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { useRecordMutations } from "@/hooks/use-records";

export type CronJobRecord = RecordModel & {
  name: string;
  schedule: string;
  job: string;
  enabled: boolean;
  lastRunAt?: string | null;
  lastStatus?: string | null;
};

interface CronJobDrawerProps {
  job: CronJobRecord | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** Create/edit form for one `_cron_jobs` row. Uses the same
 * `useRecordMutations` hook the generic records UI does — a cron job is
 * still just a record, it just gets a friendlier editor than the raw
 * schema-driven one. */
export function CronJobDrawer({ job, open, onOpenChange }: CronJobDrawerProps) {
  const isNew = job === null;
  const [name, setName] = useState("");
  const [schedule, setSchedule] = useState("");
  const [jobKey, setJobKey] = useState("");
  const [enabled, setEnabled] = useState(true);
  const { create, update } = useRecordMutations("_cron_jobs");

  useEffect(() => {
    if (!open) return;
    setName(job?.name ?? "");
    setSchedule(job?.schedule ?? "* * * * *");
    setJobKey(job?.job ?? "log_stats");
    setEnabled(job?.enabled ?? true);
  }, [open, job]);

  const pending = create.isPending || update.isPending;

  async function handleSubmit() {
    const data = { name, schedule, job: jobKey, enabled };
    try {
      if (isNew) {
        await create.mutateAsync(data);
        toast.success(`Cron job "${name}" created`);
      } else {
        await update.mutateAsync({ id: job.id, data });
        toast.success(`Cron job "${name}" saved`);
      }
      onOpenChange(false);
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Failed to save cron job");
    }
  }

  return (
    <Drawer open={open} onOpenChange={onOpenChange} title={isNew ? "New cron job" : "Edit cron job"} width={420}>
      <div className="flex flex-col gap-4">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="cron-name">Name</Label>
          <Input id="cron-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="nightly-stats" />
        </div>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="cron-schedule">Schedule (cron expression)</Label>
          <Input
            id="cron-schedule"
            value={schedule}
            onChange={(e) => setSchedule(e.target.value)}
            placeholder="* * * * *"
            className="font-mono"
          />
          <p className="text-[12px] text-muted-foreground">Standard 5-field cron syntax, checked every 30s.</p>
        </div>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="cron-job">Job</Label>
          <Input
            id="cron-job"
            value={jobKey}
            onChange={(e) => setJobKey(e.target.value)}
            placeholder="log_stats"
            className="font-mono"
          />
          <p className="text-[12px] text-muted-foreground">
            Name of a job body registered in the server binary's job registry.
          </p>
        </div>

        <div className="flex items-center justify-between rounded-lg border border-border px-3 py-2">
          <Label htmlFor="cron-enabled" className="text-[13px] font-normal">
            Enabled
          </Label>
          <Switch id="cron-enabled" checked={enabled} onCheckedChange={setEnabled} />
        </div>

        {!isNew && (job?.lastRunAt || job?.lastStatus) ? (
          <div className="rounded-lg border border-border px-3 py-2 text-[12px] text-muted-foreground">
            {job?.lastRunAt ? <p>Last run: {new Date(job.lastRunAt).toLocaleString()}</p> : null}
            {job?.lastStatus ? <p>Last status: {job.lastStatus}</p> : null}
          </div>
        ) : null}
      </div>

      <div className="mt-6 flex justify-end">
        <LoadingButton
          onAction={handleSubmit}
          pendingLabel="Saving…"
          successLabel="Saved"
          errorLabel="Failed"
          disabled={pending || name.trim().length === 0 || schedule.trim().length === 0 || jobKey.trim().length === 0}
          className="!border-primary !bg-primary !px-4 !text-primary-foreground hover:!bg-primary/90 dark:!border-primary dark:!bg-primary dark:!text-primary-foreground dark:hover:!bg-primary/90"
        >
          {isNew ? "Create job" : "Save changes"}
        </LoadingButton>
      </div>
    </Drawer>
  );
}
