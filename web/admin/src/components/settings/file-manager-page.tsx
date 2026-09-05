import { useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ChevronRight, Copy, Download, FilePlus2, Folder, FolderOpen, FolderPlus, Pencil, Trash2, Upload } from "lucide-react";
import { cb, describeFailure } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
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
import { Breadcrumb, BreadcrumbItem, BreadcrumbLink, BreadcrumbList, BreadcrumbPage, BreadcrumbSeparator } from "@/components/ui/breadcrumb";
import { Button } from "@/components/ui/button";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "@/components/ui/context-menu";
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
import { SettingsPage } from "@/components/settings/settings-form";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

/** As `crate::routes::file_manager::list` returns it: `folders` are full
 * prefixes (each already ending in `/`) ready to pass straight back as
 * the next `prefix`, `files` are the leaves sitting directly under the
 * requested prefix — one level, not a recursive walk. */
type ListResponse = { folders: string[]; files: Array<{ key: string; size: number; lastModified: string }> };

/** The server writes PocketBase's datetime form (a space, not a `T`),
 * which `new Date()` does not parse in every browser. */
function parseServerDate(value: string): Date {
  return new Date(value.replace(" ", "T"));
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** exponent;
  return `${exponent === 0 ? value : value.toFixed(1)} ${units[exponent]}`;
}

/** The last non-empty segment of a key or folder prefix — what to show in
 * the table/breadcrumb instead of the whole path. */
function baseName(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const slash = trimmed.lastIndexOf("/");
  return slash === -1 ? trimmed : trimmed.slice(slash + 1);
}

/** Object bytes with the same bearer token every other API call uses,
 * which a plain `<a href>` navigation can't attach. Shared by downloads
 * (which hand the bytes to the browser as a file) and by rename/duplicate
 * (which re-upload them under a different key) — there is no server-side
 * copy or rename endpoint, so client-side download-then-reupload is the
 * primitive both build on. `cb.buildURL` (not a hand-rolled
 * `${cb.baseURL}/...` template) matters here: `cb.baseURL` is configured
 * as `"/"` (see `lib/api.ts`), so naively prefixing it produces
 * `"//api/..."` — a protocol-relative URL the browser resolves against a
 * host literally named `api`, not this origin. */
async function fetchObjectBytes(key: string): Promise<Blob> {
  const headers: Record<string, string> = {};
  if (cb.authStore.token) headers.authorization = `Bearer ${cb.authStore.token}`;
  const url = cb.buildURL(`/api/storage/objects/download?key=${encodeURIComponent(key)}`);
  const response = await fetch(url, { headers });
  if (!response.ok) {
    throw new Error(`download failed with status ${response.status}`);
  }
  return response.blob();
}

async function downloadObject(key: string): Promise<void> {
  const blob = await fetchObjectBytes(key);
  const objectUrl = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = objectUrl;
  link.download = baseName(key);
  link.click();
  URL.revokeObjectURL(objectUrl);
}

/** Writes `data` at `key`. Mirrors what the upload button already posts,
 * just callable for a marker file or a rename/duplicate's re-upload
 * instead of only a user-picked `File`. `requestKey: null` matters here:
 * a folder rename fires several of these in a row to the same URL, and
 * the SDK's default auto-cancellation (same rule `checkHealth` in
 * `lib/api.ts` works around) would otherwise abort the previous one
 * before it finishes. */
async function putObject(key: string, data: Blob, filename: string): Promise<void> {
  const form = new FormData();
  form.append("key", key);
  form.append("file", data, filename);
  await cb.send<void>("/api/storage/objects", { method: "POST", body: form, requestKey: null });
}

async function deleteObject(key: string): Promise<void> {
  await cb.send<void>("/api/storage/objects", { method: "DELETE", query: { key }, requestKey: null });
}

/** Every object under `prefix`, walking into subfolders — `list` itself
 * only returns one level, so a folder rename (which has to move every
 * object beneath it) recurses over it client-side. Cheap enough for an
 * operator tool; this is not a hot path. */
async function listAllKeysUnder(prefix: string): Promise<string[]> {
  const page = await cb.send<ListResponse>("/api/storage/objects", { method: "GET", query: { prefix }, requestKey: null });
  const nested = await Promise.all(page.folders.map((folder) => listAllKeysUnder(folder)));
  return [...page.files.map((file) => file.key), ...nested.flat()];
}

/** `"report.csv"` next to an existing `"report.csv"` becomes
 * `"report copy.csv"`, then `"report copy 2.csv"`, `"report copy 3.csv"` —
 * the same numbering scheme most file managers use, checked against the
 * keys already listed at this level so a duplicate never silently
 * overwrites another file. */
function nextCopyKey(key: string, existingKeys: Set<string>): string {
  const name = baseName(key);
  const dir = key.slice(0, key.length - name.length);
  const dot = name.lastIndexOf(".");
  const stem = dot > 0 ? name.slice(0, dot) : name;
  const ext = dot > 0 ? name.slice(dot) : "";
  let candidate = `${dir}${stem} copy${ext}`;
  for (let n = 2; existingKeys.has(candidate); n++) {
    candidate = `${dir}${stem} copy ${n}${ext}`;
  }
  return candidate;
}

/** What the rename/duplicate/new-folder dialog is currently doing — one
 * shared shape instead of three near-identical dialogs. */
type NameAction =
  | { kind: "new-folder" }
  | { kind: "rename"; target: { type: "file" | "folder"; key: string } };

/** Raw bucket/file browser over the storage backend configured in
 * Mail & storage (`settings.s3`), independent of any record — see
 * `crate::routes::file_manager`'s doc comment for why this exists next to
 * the per-record file API and what "delete" does and doesn't clean up. */
export function FileManagerPage() {
  const queryClient = useQueryClient();
  const [prefix, setPrefix] = useState("");
  const [deleting, setDeleting] = useState<{ type: "file" | "folder"; key: string } | null>(null);
  const [nameAction, setNameAction] = useState<NameAction | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [downloadingKey, setDownloadingKey] = useState<string | null>(null);

  const { data, isLoading } = useQuery({
    queryKey: ["storage-objects", prefix],
    queryFn: () => cb.send<ListResponse>("/api/storage/objects", { method: "GET", query: { prefix } }),
  });

  function invalidate() {
    return queryClient.invalidateQueries({ queryKey: ["storage-objects"] });
  }

  const upload = useMutation({
    mutationFn: (file: File) => putObject(`${prefix}${file.name}`, file, file.name),
    onSuccess: async () => {
      await invalidate();
      toast.success("File uploaded");
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.detail || undefined });
    },
  });

  const mkdir = useMutation({
    mutationFn: (name: string) => putObject(`${prefix}${name}/.keep`, new Blob([]), ".keep"),
    onSuccess: async () => {
      await invalidate();
      toast.success("Folder created");
      setNameAction(null);
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.detail || undefined });
    },
  });

  // A file rename downloads the bytes and re-uploads them under the new
  // key; a folder rename does the same for every object underneath it,
  // then clears out every old key once every new one has landed — see
  // `listAllKeysUnder` and `putObject`'s doc comments for why there's no
  // cheaper path.
  const rename = useMutation({
    mutationFn: async ({ target, newName }: { target: { type: "file" | "folder"; key: string }; newName: string }) => {
      if (target.type === "file") {
        const destKey = `${prefix}${newName}`;
        const blob = await fetchObjectBytes(target.key);
        await putObject(destKey, blob, newName);
        await deleteObject(target.key);
        return;
      }
      const oldPrefix = target.key;
      const newPrefix = `${prefix}${newName}/`;
      const keys = await listAllKeysUnder(oldPrefix);
      for (const key of keys) {
        const destKey = `${newPrefix}${key.slice(oldPrefix.length)}`;
        const blob = await fetchObjectBytes(key);
        await putObject(destKey, blob, baseName(destKey));
      }
      for (const key of keys) await deleteObject(key);
    },
    onSuccess: async () => {
      await invalidate();
      toast.success("Renamed");
      setNameAction(null);
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.detail || undefined });
    },
  });

  const duplicate = useMutation({
    mutationFn: async (key: string) => {
      const existingKeys = new Set((data?.files ?? []).map((file) => file.key));
      const destKey = nextCopyKey(key, existingKeys);
      const blob = await fetchObjectBytes(key);
      await putObject(destKey, blob, baseName(destKey));
    },
    onSuccess: async () => {
      await invalidate();
      toast.success("File duplicated");
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.detail || undefined });
    },
  });

  const remove = useMutation({
    mutationFn: async (target: { type: "file" | "folder"; key: string }) =>
      target.type === "file" ? deleteObject(target.key) : Promise.all((await listAllKeysUnder(target.key)).map(deleteObject)),
    onSuccess: async () => {
      await invalidate();
      toast.success("Deleted");
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.detail || undefined });
    },
  });

  async function handleDownload(key: string) {
    setDownloadingKey(key);
    try {
      await downloadObject(key);
    } catch (error) {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.detail || undefined });
    } finally {
      setDownloadingKey(null);
    }
  }

  function handleFileSelected(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (file) upload.mutate(file);
  }

  // Breadcrumb entries: "Bucket root" plus one crumb per path segment,
  // each linking back to the prefix it represents.
  const segments = prefix.split("/").filter(Boolean);
  const crumbs = segments.map((segment, i) => ({
    label: segment,
    prefix: `${segments.slice(0, i + 1).join("/")}/`,
  }));

  const folders = data?.folders ?? [];
  const files = data?.files ?? [];
  const isEmpty = !isLoading && folders.length === 0 && files.length === 0;
  const takenNames = new Set([...folders.map((f) => baseName(f)), ...files.map((f) => baseName(f.key))]);

  const item = settingsItemFor("/settings/file-manager")!;
  return (
    <SettingsPage title={item.label} description={item.description} width="wide">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <Breadcrumb>
          <BreadcrumbList>
            <BreadcrumbItem>
              {prefix === "" ? (
                <BreadcrumbPage>Bucket root</BreadcrumbPage>
              ) : (
                <BreadcrumbLink asChild>
                  <button type="button" onClick={() => setPrefix("")}>
                    Bucket root
                  </button>
                </BreadcrumbLink>
              )}
            </BreadcrumbItem>
            {crumbs.map((crumb, i) => (
              <span key={crumb.prefix} className="flex items-center gap-1.5">
                <BreadcrumbSeparator />
                <BreadcrumbItem>
                  {i === crumbs.length - 1 ? (
                    <BreadcrumbPage>{crumb.label}</BreadcrumbPage>
                  ) : (
                    <BreadcrumbLink asChild>
                      <button type="button" onClick={() => setPrefix(crumb.prefix)}>
                        {crumb.label}
                      </button>
                    </BreadcrumbLink>
                  )}
                </BreadcrumbItem>
              </span>
            ))}
          </BreadcrumbList>
        </Breadcrumb>
        <div className="flex items-center gap-2">
          <input ref={fileInputRef} type="file" className="hidden" onChange={handleFileSelected} />
          <Button variant="outline" onClick={() => setNameAction({ kind: "new-folder" })}>
            <FolderPlus className="size-3.5" />
            New folder
          </Button>
          <Button variant="outline" onClick={() => fileInputRef.current?.click()} disabled={upload.isPending}>
            {upload.isPending ? <Spinner /> : <Upload className="size-3.5" />}
            {upload.isPending ? "Uploading…" : "Upload file"}
          </Button>
        </div>
      </div>

      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Name</TableHead>
            <TableHead className="w-[100px]">Size</TableHead>
            <TableHead className="w-[180px]">Modified</TableHead>
            <TableHead className="w-[110px]" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {folders.map((folder) => (
            <ContextMenu key={folder}>
              <ContextMenuTrigger asChild>
                <TableRow className="cursor-pointer" onClick={() => setPrefix(folder)}>
                  <TableCell className="font-mono text-xs">
                    <span className="flex items-center gap-1.5">
                      <Folder className="size-3.5 text-muted-foreground" />
                      {baseName(folder)}/
                    </span>
                  </TableCell>
                  <TableCell className="text-xs text-muted-foreground">—</TableCell>
                  <TableCell className="text-xs text-muted-foreground">—</TableCell>
                  <TableCell>
                    <div className="flex justify-end">
                      <ChevronRight className="size-3.5 text-muted-foreground" />
                    </div>
                  </TableCell>
                </TableRow>
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuItem
                  onSelect={() => setNameAction({ kind: "rename", target: { type: "folder", key: folder } })}
                >
                  <Pencil />
                  Rename
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem
                  variant="destructive"
                  onSelect={() => setDeleting({ type: "folder", key: folder })}
                >
                  <Trash2 />
                  Delete
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          ))}
          {files.map((file) => (
            <ContextMenu key={file.key}>
              <ContextMenuTrigger asChild>
                <TableRow>
                  <TableCell className="font-mono text-xs">{baseName(file.key)}</TableCell>
                  <TableCell className="text-xs text-muted-foreground">{formatBytes(file.size)}</TableCell>
                  <TableCell className="text-xs text-muted-foreground">
                    {parseServerDate(file.lastModified).toLocaleString()}
                  </TableCell>
                  <TableCell>
                    <div className="flex justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Download ${file.key}`}
                        title="Download"
                        onClick={() => handleDownload(file.key)}
                        disabled={downloadingKey === file.key}
                      >
                        {downloadingKey === file.key ? <Spinner /> : <Download className="size-3.5" />}
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Delete ${file.key}`}
                        title="Delete"
                        onClick={() => setDeleting({ type: "file", key: file.key })}
                      >
                        <Trash2 className="size-3.5 text-destructive" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuItem
                  onSelect={() => setNameAction({ kind: "rename", target: { type: "file", key: file.key } })}
                >
                  <Pencil />
                  Rename
                </ContextMenuItem>
                <ContextMenuItem onSelect={() => duplicate.mutate(file.key)}>
                  <Copy />
                  Duplicate
                </ContextMenuItem>
                <ContextMenuItem onSelect={() => handleDownload(file.key)}>
                  <Download />
                  Download
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem variant="destructive" onSelect={() => setDeleting({ type: "file", key: file.key })}>
                  <Trash2 />
                  Delete
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
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
          {isEmpty ? (
            <TableRow>
              <TableCell colSpan={4} className="py-8">
                <Empty>
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <FolderOpen />
                    </EmptyMedia>
                    <EmptyTitle>Nothing here</EmptyTitle>
                    <EmptyDescription>
                      This {prefix === "" ? "bucket" : "folder"} has no objects yet. Upload a file or create a
                      folder above.
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              </TableCell>
            </TableRow>
          ) : null}
        </TableBody>
      </Table>

      {nameAction ? (
        <NameDialog
          action={nameAction}
          takenNames={takenNames}
          pending={nameAction.kind === "new-folder" ? mkdir.isPending : rename.isPending}
          onCancel={() => setNameAction(null)}
          onSubmit={(name) => {
            if (nameAction.kind === "new-folder") {
              mkdir.mutate(name);
            } else {
              rename.mutate({ target: nameAction.target, newName: name });
            }
          }}
        />
      ) : null}

      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete this {deleting?.type === "folder" ? "folder" : "file"}?</AlertDialogTitle>
            <AlertDialogDescription>
              <span className="font-mono">{deleting ? baseName(deleting.key) : ""}</span>
              {deleting?.type === "folder" ? " and everything inside it" : ""} will be removed from storage
              permanently. This does <strong>not</strong> update any record that references a file here — the
              record will point at a now-missing file. This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                const target = deleting;
                setDeleting(null);
                if (target) remove.mutate(target);
              }}
            >
              Delete {deleting?.type === "folder" ? "folder" : "file"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </SettingsPage>
  );
}

/** One dialog for both "new folder" and "rename" — same shape (a single
 * name field, validated the same way), just a different title/verb and a
 * different pre-filled value. */
function NameDialog({
  action,
  takenNames,
  pending,
  onCancel,
  onSubmit,
}: {
  action: NameAction;
  takenNames: Set<string>;
  pending: boolean;
  onCancel: () => void;
  onSubmit: (name: string) => void;
}) {
  const initial = action.kind === "rename" ? baseName(action.target.key) : "";
  const [name, setName] = useState(initial);

  const trimmed = name.trim();
  const collides = trimmed !== initial && takenNames.has(trimmed);
  const invalid = trimmed.length === 0 || trimmed.includes("/") || collides;

  const title = action.kind === "new-folder" ? "New folder" : action.target.type === "folder" ? "Rename folder" : "Rename file";
  const submitLabel = action.kind === "new-folder" ? "Create" : "Rename";

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {action.kind === "new-folder" ? (
            <DialogDescription>Object keys are flat — this creates an empty marker object so it lists as a folder.</DialogDescription>
          ) : null}
        </DialogHeader>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="object-name">Name</Label>
          <Input
            id="object-name"
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !invalid) onSubmit(trimmed);
            }}
            placeholder={action.kind === "new-folder" ? "assets" : undefined}
          />
          {trimmed.includes("/") ? <p className="text-xs text-destructive">Names can't contain "/".</p> : null}
          {collides ? <p className="text-xs text-destructive">Something with that name already exists here.</p> : null}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          <Button disabled={invalid || pending} onClick={() => onSubmit(trimmed)}>
            {pending ? <Spinner className="size-3.5" /> : action.kind === "new-folder" ? <FilePlus2 className="size-3.5" /> : null}
            {submitLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
