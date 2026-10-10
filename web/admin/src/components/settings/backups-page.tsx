import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Archive, Download, HardDrive, RotateCcw, Trash2, TriangleAlert, Upload } from "lucide-react";
import { cb, checkBackupCapability, describeFailure, parseServerDate, superuserAuth } from "@/lib/api";
import { describeSchedule } from "@/lib/cron";
import { SETTINGS_GROUPS } from "@/lib/settings-nav";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { relativeTime, TimeAgo } from "@/components/settings/time-ago";
import { TypedConfirmDialog } from "@/components/settings/typed-confirm-dialog";
import { Spinner } from "@/components/ui/spinner";
import { Skeleton } from "@/components/ui/skeleton";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { StatCard, StatusBadge } from "@/components/layout/stat-card";
import {
  NumberSetting,
  SettingRow,
  SettingsPage,
  SettingsSaveBar,
  SettingsSection,
  TextSetting,
} from "@/components/settings/settings-form";
import { useSaveShortcut } from "@/components/settings/use-save-shortcut";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const ITEM = SETTINGS_GROUPS.flatMap((g) => g.tabs).find((t) => t.legacyPath === "/settings/backups")!;

/** The slice of settings this page owns. */
type ScheduleDraft = Pick<ServerSettings["backups"], "cron" | "cronMaxKeep">;

function scheduleDraftOf(settings: ServerSettings): ScheduleDraft {
  return { cron: settings.backups.cron, cronMaxKeep: settings.backups.cronMaxKeep };
}

/** A loose sanity check, not a real cron parser — 5 whitespace-separated
 * fields, same shape `_cron_jobs.schedule` and every other cron field in
 * this dashboard expects. Empty is valid (means "no scheduled backups"). */
function validateCron(cron: string): string | null {
  if (!cron.trim()) return null;
  const fields = cron.trim().split(/\s+/);
  if (fields.length !== 5) return "A cron schedule needs 5 fields: minute hour day month weekday";
  return null;
}

const SCHEDULE_PRESETS = [
  { label: "Daily 03:00", cron: "0 3 * * *" },
  { label: "Weekly (Sun 03:00)", cron: "0 3 * * 0" },
  { label: "Every 6 hours", cron: "0 */6 * * *" },
  { label: "Off", cron: "" },
];

/** As the server returns it — the server names the file `key` and its
 * timestamp `modified`, not `name`/`created`. */
type BackupInfo = {
  key: string;
  size: number;
  modified: string;
};

/** The outcome of the most recent `settings.backups.cron` run, from
 * `GET /api/backups/storage-info` — absent until the cron job has fired
 * at least once. */
type LastScheduledBackup = {
  at: string;
  ok: boolean;
  message?: string;
};

type StorageInfo = { driver: "local" | "s3"; location: string; lastScheduled?: LastScheduledBackup };

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value.toFixed(1)} ${units[unitIndex]}`;
}

/** Backup downloads require a superuser bearer token, which a plain
 * `<a href>` navigation can't attach — fetch the bytes with the same
 * `Authorization` header every other API call uses, then hand the
 * browser a blob URL to save. */
async function downloadBackup(name: string): Promise<void> {
  const headers: Record<string, string> = {};
  if (superuserAuth.token) headers.authorization = `Bearer ${superuserAuth.token}`;
  const response = await fetch(cb.buildURL(`/api/backups/${encodeURIComponent(name)}/download`), { headers });
  if (!response.ok) {
    throw new Error(`download failed with status ${response.status}`);
  }
  const blob = await response.blob();
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  URL.revokeObjectURL(url);
}

/** After a restore the server re-execs itself. Poll `/api/health` until it has
 * gone away and come back, then reload so the dashboard talks to the restored
 * instance. Gives up after a minute and says so. */
function useReloadWhenServerReturns(active: boolean) {
  const [timedOut, setTimedOut] = useState(false);
  useEffect(() => {
    if (!active) return;
    let sawDown = false;
    const startedAt = Date.now();
    const timer = window.setInterval(async () => {
      try {
        const response = await fetch(cb.buildURL("/api/health"), { cache: "no-store" });
        if (response.ok && sawDown) window.location.reload();
        if (!response.ok) sawDown = true;
      } catch {
        sawDown = true;
      }
      if (Date.now() - startedAt > 60_000) {
        window.clearInterval(timer);
        setTimedOut(true);
      }
    }, 1500);
    return () => window.clearInterval(timer);
  }, [active]);
  return timedOut;
}

function toastFailure(error: unknown, title?: string) {
  const failure = describeFailure(error);
  toast.error(title ?? failure.title, { description: failure.detail || failure.serverMessage || undefined });
}

/** Triggers, lists, downloads, and deletes backups — SQLite via `VACUUM
 * INTO`, Postgres via `pg_dump`/`pg_restore` (see
 * `crate::routes::backups`'s doc comment). */
export function BackupsPage() {
  const queryClient = useQueryClient();
  const [deleting, setDeleting] = useState<BackupInfo | null>(null);
  const [restoring, setRestoring] = useState<BackupInfo | null>(null);
  const [downloadingKey, setDownloadingKey] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const { data: settings } = useSettings();
  const saveSchedule = useSettingsMutation();
  const [scheduleDraft, setScheduleDraft] = useState<ScheduleDraft | null>(null);
  if (settings && scheduleDraft === null) setScheduleDraft(scheduleDraftOf(settings));
  const scheduleDirty =
    scheduleDraft && settings ? JSON.stringify(scheduleDraft) !== JSON.stringify(scheduleDraftOf(settings)) : false;
  const cronError = scheduleDraft ? validateCron(scheduleDraft.cron) : null;
  const scheduleErrors = useMemo(() => {
    if (!scheduleDraft) return [];
    const errors: string[] = [];
    const error = validateCron(scheduleDraft.cron);
    if (error) errors.push(error);
    if (scheduleDraft.cronMaxKeep < 0) errors.push("Backups to keep can't be negative");
    return errors;
  }, [scheduleDraft]);
  function submitSchedule() {
    if (!scheduleDraft || scheduleErrors.length > 0) return;
    saveSchedule.mutate(
      { backups: { ...settings?.backups, ...scheduleDraft } },
      {
        onSuccess: () => {
          toast.success("Backup schedule saved");
          void queryClient.invalidateQueries({ queryKey: ["backups", "storage-info"] });
        },
        onError: (error) => toastFailure(error),
      },
    );
  }
  useSaveShortcut({
    dirty: scheduleDirty,
    canSave: scheduleErrors.length === 0 && !saveSchedule.isPending,
    onSave: submitSchedule,
  });

  const {
    data: backups,
    isPending,
    isError,
    error: listError,
    refetch,
  } = useQuery({
    queryKey: ["backups"],
    queryFn: () => cb.send<BackupInfo[]>("/api/backups", { method: "GET" }),
  });
  const { data: storageInfo } = useQuery({
    queryKey: ["backups", "storage-info"],
    queryFn: () => cb.send<StorageInfo>("/api/backups/storage-info", { method: "GET" }),
    staleTime: 5 * 60 * 1000,
  });
  // False when backup storage itself isn't usable (e.g. a misconfigured
  // S3 target) — SQLite and Postgres both back up fine otherwise.
  const { data: canBackup } = useQuery({
    queryKey: ["backups", "capability"],
    queryFn: checkBackupCapability,
    staleTime: 5 * 60 * 1000,
  });

  function invalidate() {
    return queryClient.invalidateQueries({ queryKey: ["backups"] });
  }

  const create = useMutation({
    mutationFn: () => cb.send<BackupInfo>("/api/backups", { method: "POST", body: {} }),
    onSuccess: async (backup) => {
      await invalidate();
      toast.success(`Backup "${backup.key}" created`);
    },
    onError: (error) => toastFailure(error),
  });

  const download = useMutation({
    mutationFn: downloadBackup,
    onMutate: (key) => setDownloadingKey(key),
    onSettled: () => setDownloadingKey(null),
    onError: (error) => toastFailure(error),
  });

  const remove = useMutation({
    mutationFn: (name: string) => cb.send<void>(`/api/backups/${encodeURIComponent(name)}`, { method: "DELETE" }),
    onSuccess: async () => {
      await invalidate();
      toast.success("Backup deleted");
    },
    onError: (error) => toastFailure(error),
  });

  // `cb.backups.upload()` is the SDK's own helper — it multipart-encodes
  // `{ file }` exactly the way `crate::routes::backups::upload` expects
  // (a `file` part plus an optional `name` part), so there's no reason to
  // hand-roll a `FormData` here the way the other actions build raw
  // `cb.send()` calls.
  const upload = useMutation({
    mutationFn: (file: File) => cb.admin.backups.upload(file),
    onSuccess: async () => {
      await invalidate();
      toast.success("Backup uploaded");
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.fields.file ?? failure.detail ?? undefined });
    },
  });

  // A restore closes every open database handle, swaps the data directory,
  // and re-execs the server process (see the doc comment on
  // `crate::routes::backups::restore`) — so a success here doesn't mean the
  // restore finished, only that it started. The connection drops out from
  // under whatever screen is open next; `useReloadWhenServerReturns` waits
  // for the server to come back and reloads the dashboard.
  const restore = useMutation({
    mutationFn: (key: string) => cb.admin.backups.restore(key),
    onError: (error) => toastFailure(error),
  });
  const restartTimedOut = useReloadWhenServerReturns(restore.isSuccess);

  function handleFileSelected(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (file) upload.mutate(file);
  }

  const totalSize = (backups ?? []).reduce((sum, b) => sum + b.size, 0);
  const sorted = useMemo(
    () => [...(backups ?? [])].sort((a, b) => parseServerDate(b.modified).getTime() - parseServerDate(a.modified).getTime()),
    [backups],
  );
  const latest = sorted[0];
  const scheduleText = settings?.backups.cron.trim()
    ? describeSchedule(settings.backups.cron)
    : null;

  if (restore.isSuccess) {
    return (
      <SettingsPage title={ITEM.label} description={ITEM.description} width="wide">
        <EmptyState
          icon={restartTimedOut ? <TriangleAlert /> : <Spinner />}
          title={restartTimedOut ? "The server hasn't come back yet" : "Restoring backup…"}
          description={
            restartTimedOut
              ? "The restore was accepted but the server is still unreachable. Check the process is running, then reload this page."
              : "The server is restarting to load it. This page reloads on its own when the server is back."
          }
          action={
            restartTimedOut ? (
              <Button size="sm" onClick={() => window.location.reload()}>
                Reload
              </Button>
            ) : undefined
          }
        />
      </SettingsPage>
    );
  }

  if (canBackup === false) {
    return (
      <SettingsPage title={ITEM.label} description={ITEM.description} width="wide">
        <EmptyState
          icon={<Archive />}
          title="Backups aren't available"
          description={
            <>
              Backup storage isn't configured correctly — check the S3 settings under{" "}
              <code className="font-mono">settings.backups.s3</code>, or the server logs for why the local backups
              directory couldn't be used.
            </>
          }
        />
      </SettingsPage>
    );
  }

  const lastScheduled = storageInfo?.lastScheduled;

  return (
    <SettingsPage
      title={ITEM.label}
      description={ITEM.description}
      width="wide"
      action={
        <>
          <input
            ref={fileInputRef}
            type="file"
            accept=".zip"
            className="hidden"
            aria-label="Backup archive to upload"
            onChange={handleFileSelected}
          />
          <Button variant="outline" size="sm" onClick={() => fileInputRef.current?.click()} disabled={upload.isPending}>
            {upload.isPending ? <Spinner /> : <Upload />}
            {upload.isPending ? "Uploading…" : "Upload"}
          </Button>
          <Button size="sm" onClick={() => create.mutate()} disabled={create.isPending}>
            {create.isPending ? <Spinner /> : <Archive />}
            {create.isPending ? "Creating…" : "Create backup"}
          </Button>
        </>
      }
    >
      <div className="grid grid-cols-2 gap-2 lg:grid-cols-4">
        <StatCard
          index={0}
          label="Backups"
          value={backups?.length ?? 0}
          loading={isPending}
          hint={backups ? formatBytes(totalSize) + " total" : undefined}
          icon={<Archive />}
        />
        <StatCard
          index={1}
          label="Latest backup"
          value={latest ? relativeTime(parseServerDate(latest.modified)) : "None"}
          loading={isPending}
          hint={latest ? latest.key : "Create one to get started"}
          tone={!isPending && !latest ? "warning" : "neutral"}
        />
        <StatCard
          index={2}
          label="Schedule"
          value={scheduleText ? "On" : "Off"}
          loading={!settings}
          hint={scheduleText ?? "No automatic backups"}
        />
        <StatCard
          index={3}
          label="Last scheduled run"
          value={lastScheduled ? (lastScheduled.ok ? "Succeeded" : "Failed") : "—"}
          tone={lastScheduled ? (lastScheduled.ok ? "success" : "danger") : "neutral"}
          loading={!storageInfo}
          hint={lastScheduled ? relativeTime(parseServerDate(lastScheduled.at)) : "Hasn't run yet"}
        />
      </div>

      {lastScheduled && !lastScheduled.ok && lastScheduled.message ? (
        <p role="alert" className="flex items-start gap-2 rounded-lg border border-destructive/25 bg-destructive/5 px-3 py-2 text-sm text-destructive">
          <TriangleAlert className="mt-0.5 size-4 shrink-0" />
          <span>
            The last scheduled backup failed: <span className="font-mono text-xs">{lastScheduled.message}</span>
          </span>
        </p>
      ) : null}

      <section className="flex min-w-0 flex-col gap-2">
        <div className="flex flex-wrap items-end justify-between gap-2">
          <div className="flex flex-col">
            <h2 className="text-sm font-medium">Snapshots</h2>
            {storageInfo ? (
              <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <HardDrive className="size-3" aria-hidden />
                Stored on{" "}
                <span className="font-mono text-foreground">
                  {storageInfo.driver === "s3" ? `S3 (${storageInfo.location})` : `local disk (${storageInfo.location})`}
                </span>
              </p>
            ) : null}
          </div>
        </div>

        {isPending ? (
          <ListSkeleton rows={3} />
        ) : isError ? (
          <ErrorState title="Couldn't load backups" error={listError} onRetry={() => void refetch()} compact />
        ) : sorted.length === 0 && !create.isPending ? (
          <EmptyState
            compact
            icon={<Archive />}
            title="No backups yet"
            description="A backup is a point-in-time snapshot of the database you can download or restore. Create one now, or upload an existing archive."
            action={
              <>
                <Button size="sm" onClick={() => create.mutate()} disabled={create.isPending}>
                  <Archive />
                  Create backup
                </Button>
                <Button variant="outline" size="sm" onClick={() => fileInputRef.current?.click()}>
                  <Upload />
                  Upload
                </Button>
              </>
            }
          />
        ) : (
          <div className="overflow-hidden rounded-lg border border-border bg-card">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead className="w-24 text-right">Size</TableHead>
                  <TableHead className="w-44">Created</TableHead>
                  <TableHead className="w-28" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {create.isPending ? (
                  <TableRow aria-busy="true">
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      <span className="inline-flex items-center gap-2">
                        <Spinner /> Creating backup…
                      </span>
                    </TableCell>
                    <TableCell>
                      <Skeleton className="ml-auto h-3 w-12" />
                    </TableCell>
                    <TableCell>
                      <Skeleton className="h-3 w-24" />
                    </TableCell>
                    <TableCell />
                  </TableRow>
                ) : null}
                {sorted.map((backup, index) => {
                  const created = parseServerDate(backup.modified);
                  return (
                    <TableRow key={backup.key}>
                      <TableCell className="font-mono text-xs">
                        <span className="inline-flex items-center gap-2">
                          {backup.key}
                          {index === 0 ? <StatusBadge tone="success">Latest</StatusBadge> : null}
                        </span>
                      </TableCell>
                      <TableCell className="font-tabular text-right text-xs text-muted-foreground">
                        {formatBytes(backup.size)}
                      </TableCell>
                      <TableCell className="text-xs text-muted-foreground">
                        <TimeAgo value={backup.modified} />
                        <span className="ml-1.5 hidden text-muted-foreground/70 xl:inline">
                          · {created.toLocaleString()}
                        </span>
                      </TableCell>
                      <TableCell>
                        <div className="flex justify-end gap-0.5">
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            aria-label={`Download ${backup.key}`}
                            title="Download"
                            disabled={downloadingKey === backup.key}
                            onClick={() => download.mutate(backup.key)}
                          >
                            {downloadingKey === backup.key ? <Spinner /> : <Download />}
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            aria-label={`Restore ${backup.key}`}
                            title="Restore"
                            onClick={() => setRestoring(backup)}
                          >
                            <RotateCcw />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            aria-label={`Delete ${backup.key}`}
                            title="Delete"
                            onClick={() => setDeleting(backup)}
                          >
                            <Trash2 className="text-destructive" />
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
      </section>

      {scheduleDraft ? (
        <SettingsSection
          title="Scheduled backups"
          description="Run a backup automatically on a cron schedule, and cap how many scheduled backups to keep."
        >
          <SettingRow
            label="Cron schedule"
            htmlFor="backups-cron"
            help={
              scheduleDraft.cron.trim() && !cronError
                ? `${describeSchedule(scheduleDraft.cron)} (UTC)`
                : "Standard 5-field cron (minute hour day month weekday), UTC. Leave blank to disable scheduled backups."
            }
            error={cronError ?? undefined}
          >
            <TextSetting
              id="backups-cron"
              value={scheduleDraft.cron}
              onChange={(cron) => setScheduleDraft((d) => (d ? { ...d, cron } : d))}
              placeholder="0 3 * * *"
              mono
              invalid={!!cronError}
            />
            <div className="flex flex-wrap gap-1.5">
              {SCHEDULE_PRESETS.map((preset) => (
                <Button
                  key={preset.label}
                  type="button"
                  variant={scheduleDraft.cron === preset.cron ? "secondary" : "outline"}
                  size="xs"
                  onClick={() => setScheduleDraft((d) => (d ? { ...d, cron: preset.cron } : d))}
                >
                  {preset.label}
                </Button>
              ))}
            </div>
          </SettingRow>
          <SettingRow
            label="Keep at most"
            htmlFor="backups-cron-max-keep"
            help="Oldest scheduled backups beyond this count are deleted automatically. Manually created and uploaded backups aren't counted or touched."
            error={scheduleDraft.cronMaxKeep < 0 ? "Can't be negative" : undefined}
          >
            <NumberSetting
              id="backups-cron-max-keep"
              min={0}
              value={scheduleDraft.cronMaxKeep}
              onChange={(cronMaxKeep) => setScheduleDraft((d) => (d ? { ...d, cronMaxKeep } : d))}
              suffix="backups"
              invalid={scheduleDraft.cronMaxKeep < 0}
            />
          </SettingRow>
        </SettingsSection>
      ) : null}

      {scheduleDraft ? (
        <SettingsSaveBar
          dirty={scheduleDirty}
          pending={saveSchedule.isPending}
          errors={scheduleErrors}
          onSave={submitSchedule}
          onReset={() => settings && setScheduleDraft(scheduleDraftOf(settings))}
        />
      ) : null}

      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete this backup?</AlertDialogTitle>
            <AlertDialogDescription>
              <span className="font-mono">{deleting?.key}</span> will be removed from{" "}
              {storageInfo?.driver === "s3" ? "S3" : "disk"} permanently. This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                const target = deleting;
                setDeleting(null);
                if (target) remove.mutate(target.key);
              }}
            >
              Delete backup
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <TypedConfirmDialog
        open={restoring !== null}
        onOpenChange={(open) => !open && setRestoring(null)}
        title="Restore this backup?"
        description={
          <>
            Every record, file, and setting will be replaced with what is in{" "}
            <span className="font-mono">{restoring?.key}</span>. Anything created since that backup is lost. The server
            restarts itself to apply it, which drops every open connection for a few seconds. This cannot be undone —
            create a fresh backup first if you may want the current data back.
          </>
        }
        confirmValue={restoring?.key ?? ""}
        confirmLabel="Restore backup"
        onConfirm={() => {
          const target = restoring;
          setRestoring(null);
          if (target) restore.mutate(target.key);
        }}
      />
    </SettingsPage>
  );
}
