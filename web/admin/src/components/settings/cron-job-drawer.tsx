import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import type { RecordModel } from "cratebase";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { useRecordMutations } from "@/hooks/use-records";
import { cb, describeFailure } from "@/lib/api";

type AvailableJob = { name: string; description: string };

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
  const { data: availableJobs = [] } = useQuery({
    queryKey: ["cron-jobs", "available"],
    queryFn: () => cb.send<{ jobs: AvailableJob[] }>("/api/plugins/cron-jobs/available").then((r) => r.jobs),
    enabled: open,
    staleTime: 5 * 60 * 1000,
  });

  useEffect(() => {
    if (!open) return;
    setName(job?.name ?? "");
    setSchedule(job?.schedule ?? "* * * * *");
    setJobKey(job?.job ?? "log_stats");
    setEnabled(job?.enabled ?? true);
  }, [open, job]);

  const pending = create.isPending || update.isPending;
  const incomplete = name.trim().length === 0 || schedule.trim().length === 0 || jobKey.trim().length === 0;

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending || incomplete) return;
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
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.detail || undefined });
    }
  }

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="gap-0 p-0 data-[side=right]:w-full data-[side=right]:sm:max-w-[420px]">
        <form onSubmit={handleSubmit} className="flex h-full min-h-0 flex-col">
          <SheetHeader>
            <SheetTitle>{isNew ? "New cron job" : "Edit cron job"}</SheetTitle>
            <SheetDescription>
              A schedule, and the name of a job body registered in the running server binary.
            </SheetDescription>
          </SheetHeader>

          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-4 pb-4">
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
          <p className="text-sm text-muted-foreground">Standard 5-field cron syntax, checked every 30s.</p>
        </div>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="cron-job">Job</Label>
          <Select value={jobKey} onValueChange={setJobKey}>
            <SelectTrigger id="cron-job" className="w-full font-mono">
              <SelectValue placeholder="Select a job…" />
            </SelectTrigger>
            <SelectContent>
              {availableJobs.map((j) => (
                <SelectItem key={j.name} value={j.name} className="font-mono">
                  {j.name}
                </SelectItem>
              ))}
              {jobKey && !availableJobs.some((j) => j.name === jobKey) ? (
                <SelectItem value={jobKey} className="font-mono">
                  {jobKey} (not in the current binary's registry)
                </SelectItem>
              ) : null}
            </SelectContent>
          </Select>
          <p className="text-sm text-muted-foreground">
            {availableJobs.find((j) => j.name === jobKey)?.description ??
              "Every job body registered in the running server binary — see crates/server/src/plugins/cron_jobs.rs to add one."}
          </p>
        </div>

        <div className="flex items-center justify-between rounded-lg border border-border px-3 py-2">
          <Label htmlFor="cron-enabled" className="text-sm font-normal">
            Enabled
          </Label>
          <Switch id="cron-enabled" checked={enabled} onCheckedChange={setEnabled} />
        </div>

            {!isNew && (job?.lastRunAt || job?.lastStatus) ? (
              <div className="rounded-lg border border-border px-3 py-2 text-sm text-muted-foreground">
                {job?.lastRunAt ? <p>Last run: {new Date(job.lastRunAt).toLocaleString()}</p> : null}
                {job?.lastStatus ? <p>Last status: {job.lastStatus}</p> : null}
              </div>
            ) : null}
          </div>

          <SheetFooter className="flex-row justify-end border-t border-border">
            <Button type="submit" disabled={pending || incomplete}>
              {pending ? <Spinner /> : null}
              {pending ? "Saving…" : isNew ? "Create job" : "Save changes"}
            </Button>
          </SheetFooter>
        </form>
      </SheetContent>
    </Sheet>
  );
}
