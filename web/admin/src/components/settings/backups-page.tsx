import { useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Archive, Download, RotateCcw, Trash2, Upload } from "lucide-react";
import { cb, describeFailure } from "@/lib/api";
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
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

/** As the server returns it — PocketBase names the file `key` and its
 * timestamp `modified`, not `name`/`created`. */
type BackupInfo = {
  key: string;
  size: number;
  modified: string;
};

/** The server writes PocketBase's datetime form (a space, not a `T`),
 * which `new Date()` does not parse in every browser — Safari returns
 * Invalid Date. Normalise before parsing. */
function parseServerDate(value: string): Date {
  return new Date(value.replace(" ", "T"));
}

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
  if (cb.authStore.token) headers.authorization = `Bearer ${cb.authStore.token}`;
  const response = await fetch(`${cb.baseURL}/api/backups/${encodeURIComponent(name)}/download`, { headers });
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

/** Triggers, lists, downloads, and deletes SQLite backups — see
 * `crate::routes::backups`'s doc comment for why this is SQLite-only. */
export function BackupsPage() {
  const queryClient = useQueryClient();
  const [pending, setPending] = useState(false);
  const [deleting, setDeleting] = useState<BackupInfo | null>(null);
  const [restoring, setRestoring] = useState<BackupInfo | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const { data: backups, isLoading } = useQuery({
    queryKey: ["backups"],
    queryFn: () => cb.send<BackupInfo[]>("/api/backups", { method: "GET" }),
  });
  const { data: storageInfo } = useQuery({
    queryKey: ["backups", "storage-info"],
    queryFn: () =>
      cb.send<{ driver: "local" | "s3"; location: string }>("/api/backups/storage-info", {
        method: "GET",
      }),
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
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.detail || undefined });
    },
  });

  const download = useMutation({
    mutationFn: downloadBackup,
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.detail || undefined });
    },
  });

  const remove = useMutation({
    mutationFn: (name: string) => cb.send<void>(`/api/backups/${encodeURIComponent(name)}`, { method: "DELETE" }),
    onSuccess: async () => {
      await invalidate();
      toast.success("Backup deleted");
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.detail || undefined });
    },
  });

  // `pb.backups.upload()` is the SDK's own helper — it multipart-encodes
  // `{ file }` exactly the way `crate::routes::backups::upload` expects
  // (a `file` part plus an optional `name` part), so there's no reason to
  // hand-roll a `FormData` here the way the other actions build raw
  // `cb.send()` calls.
  const upload = useMutation({
    mutationFn: (file: File) => cb.backups.upload({ file }),
    onSuccess: async () => {
      await invalidate();
      toast.success("Backup uploaded");
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, {
        description: failure.fields.file ?? failure.detail ?? undefined,
      });
    },
  });

  // A restore closes every open database handle, swaps the data directory,
  // and re-execs the server process (see the doc comment on
  // `crate::routes::backups::restore`) — so a success here doesn't mean the
  // restore finished, only that it started. The connection drops out from
  // under whatever screen is open next; there's nothing more to await.
  const restore = useMutation({
    mutationFn: (key: string) => cb.backups.restore(key),
    onSuccess: () => {
      toast.success("Restoring backup", {
        description: "The server is restarting to load it. This page will lose its connection briefly.",
      });
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.detail || undefined });
    },
  });

  async function handleCreate() {
    if (pending) return;
    setPending(true);
    try {
      await create.mutateAsync();
    } catch {
      // Surfaced as a toast by the mutation's own onError.
    } finally {
      setPending(false);
    }
  }

  function handleFileSelected(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (file) upload.mutate(file);
  }

  return (
    <div className="flex flex-col gap-4 p-6">
      <div className="flex items-center justify-between">
        <p className="text-sm text-muted-foreground">
          Full-database snapshots, stored alongside your uploaded files. SQLite only.
          {storageInfo ? (
            <>
              {" "}
              Currently{" "}
              <span className="font-mono text-foreground">
                {storageInfo.driver === "s3" ? `S3 (${storageInfo.location})` : `local disk (${storageInfo.location})`}
              </span>
              .
            </>
          ) : null}
        </p>
        <div className="flex items-center gap-2">
          <input
            ref={fileInputRef}
            type="file"
            accept=".zip"
            className="hidden"
            onChange={handleFileSelected}
          />
          <Button variant="outline" onClick={() => fileInputRef.current?.click()} disabled={upload.isPending}>
            {upload.isPending ? <Spinner /> : <Upload className="size-3.5" />}
            {upload.isPending ? "Uploading…" : "Upload backup"}
          </Button>
          <Button onClick={handleCreate} disabled={pending}>
            {pending ? <Spinner /> : null}
            {pending ? "Creating…" : "Create backup"}
          </Button>
        </div>
      </div>

      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Name</TableHead>
            <TableHead className="w-[100px]">Size</TableHead>
            <TableHead className="w-[180px]">Created</TableHead>
            <TableHead className="w-[130px]" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {(backups ?? []).map((backup) => (
            <TableRow key={backup.key}>
              <TableCell className="font-mono text-xs">{backup.key}</TableCell>
              <TableCell className="text-xs text-muted-foreground">{formatBytes(backup.size)}</TableCell>
              <TableCell className="text-xs text-muted-foreground">
                {parseServerDate(backup.modified).toLocaleString()}
              </TableCell>
              <TableCell>
                <div className="flex justify-end gap-1">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Restore ${backup.key}`}
                    title="Restore"
                    onClick={() => setRestoring(backup)}
                  >
                    <RotateCcw className="size-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Download ${backup.key}`}
                    onClick={() => download.mutate(backup.key)}
                  >
                    <Download className="size-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Delete ${backup.key}`}
                    onClick={() => setDeleting(backup)}
                  >
                    <Trash2 className="size-3.5 text-destructive" />
                  </Button>
                </div>
              </TableCell>
            </TableRow>
          ))}
          {isLoading ? (
            Array.from({ length: 3 }).map((_, i) => (
              <TableRow key={i}>
                <TableCell colSpan={4}>
                  <Skeleton className="h-row w-full" />
                </TableCell>
              </TableRow>
            ))
          ) : null}
          {!isLoading && (backups?.length ?? 0) === 0 ? (
            <TableRow>
              <TableCell colSpan={4} className="py-8">
                <Empty>
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <Archive />
                    </EmptyMedia>
                    <EmptyTitle>No backups yet</EmptyTitle>
                    <EmptyDescription>
                      Create one above to snapshot the database as it stands right now, or upload an existing
                      archive.
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              </TableCell>
            </TableRow>
          ) : null}
        </TableBody>
      </Table>

      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete this backup?</AlertDialogTitle>
            <AlertDialogDescription>
              <span className="font-mono">{deleting?.key}</span> will be removed from disk permanently. This
              cannot be undone.
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

      <AlertDialog open={restoring !== null} onOpenChange={(open) => !open && setRestoring(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Restore this backup?</AlertDialogTitle>
            <AlertDialogDescription>
              Every record, file, and setting will be replaced with what is in{" "}
              <span className="font-mono">{restoring?.key}</span>. The server restarts itself to apply it, which
              drops every open connection for a few seconds. This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                const target = restoring;
                setRestoring(null);
                if (target) restore.mutate(target.key);
              }}
            >
              Restore backup
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
