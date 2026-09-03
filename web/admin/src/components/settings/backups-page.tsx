import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Download, Trash2 } from "lucide-react";
import { cb } from "@/lib/api";
import { LoadingButton } from "@/components/interior/loading-button";
import { Button } from "@/components/ui/button";
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

  const { data: backups, isLoading } = useQuery({
    queryKey: ["backups"],
    queryFn: () => cb.send<BackupInfo[]>("/api/backups", { method: "GET" }),
  });
  const { data: storageInfo } = useQuery({
    queryKey: ["backups", "storage-info"],
    queryFn: () => cb.send<{ driver: "local" | "s3"; location: string }>("/api/backups/storage-info"),
    staleTime: 5 * 60 * 1000,
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
      toast.error(error instanceof Error ? error.message : "Failed to create backup");
    },
  });

  const download = useMutation({
    mutationFn: downloadBackup,
    onError: (error) => toast.error(error instanceof Error ? error.message : "Failed to download backup"),
  });

  const remove = useMutation({
    mutationFn: (name: string) => cb.send<void>(`/api/backups/${encodeURIComponent(name)}`, { method: "DELETE" }),
    onSuccess: async () => {
      await invalidate();
      toast.success("Backup deleted");
    },
    onError: (error) => toast.error(error instanceof Error ? error.message : "Failed to delete backup"),
  });

  return (
    <div className="flex flex-col gap-4 p-6">
      <div className="flex items-center justify-between">
        <p className="text-[12.5px] text-muted-foreground">
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
        <LoadingButton
          onAction={() => create.mutateAsync()}
          pendingLabel="Creating…"
          successLabel="Created"
          errorLabel="Failed"
        >
          Create backup
        </LoadingButton>
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
                    onClick={() => remove.mutate(backup.name)}
                  >
                    <Trash2 className="size-3.5 text-destructive" />
                  </Button>
                </div>
              </TableCell>
            </TableRow>
          ))}
          {!isLoading && (backups?.length ?? 0) === 0 ? (
            <TableRow>
              <TableCell colSpan={4} className="py-8 text-center text-sm text-muted-foreground">
                No backups yet.
              </TableCell>
            </TableRow>
          ) : null}
        </TableBody>
      </Table>
    </div>
  );
}
