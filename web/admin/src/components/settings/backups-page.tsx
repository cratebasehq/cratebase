import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Archive, Download, Trash2 } from "lucide-react";
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

type BackupInfo = {
  name: string;
  size: number;
  created: string;
};

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
  const response = await fetch(`${cb.baseUrl}/api/backups/${encodeURIComponent(name)}/download`, { headers });
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

  const { data: backups, isLoading } = useQuery({
    queryKey: ["backups"],
    queryFn: () => cb.send<BackupInfo[]>("/api/backups", { method: "GET" }),
  });

  function invalidate() {
    return queryClient.invalidateQueries({ queryKey: ["backups"] });
  }

  const create = useMutation({
    mutationFn: () => cb.send<BackupInfo>("/api/backups", { method: "POST", body: {} }),
    onSuccess: async (backup) => {
      await invalidate();
      toast.success(`Backup "${backup.name}" created`);
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

  return (
    <div className="flex flex-col gap-4 p-6">
      <div className="flex items-center justify-between">
        <p className="text-sm text-muted-foreground">
          Full-database snapshots, stored alongside your uploaded files. SQLite only.
        </p>
        <Button onClick={handleCreate} disabled={pending}>
          {pending ? <Spinner /> : null}
          {pending ? "Creating…" : "Create backup"}
        </Button>
      </div>

      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Name</TableHead>
            <TableHead className="w-[100px]">Size</TableHead>
            <TableHead className="w-[180px]">Created</TableHead>
            <TableHead className="w-[100px]" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {(backups ?? []).map((backup) => (
            <TableRow key={backup.name}>
              <TableCell className="font-mono text-xs">{backup.name}</TableCell>
              <TableCell className="text-xs text-muted-foreground">{formatBytes(backup.size)}</TableCell>
              <TableCell className="text-xs text-muted-foreground">
                {new Date(backup.created).toLocaleString()}
              </TableCell>
              <TableCell>
                <div className="flex justify-end gap-1">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Download ${backup.name}`}
                    onClick={() => download.mutate(backup.name)}
                  >
                    <Download className="size-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Delete ${backup.name}`}
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
                      Create one above to snapshot the database as it stands right now.
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
              <span className="font-mono">{deleting?.name}</span> will be removed from disk permanently. This
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
                if (target) remove.mutate(target.name);
              }}
            >
              Delete backup
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
