import { Fragment, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  Copy,
  Download,
  File as FileIcon,
  FileArchive,
  FilePlus2,
  FileAudio,
  FileImage,
  FileText,
  FileVideo,
  Folder,
  FolderPlus,
  FolderOpen,
  LayoutGrid,
  List,
  Pencil,
  Trash2,
  Upload,
} from "lucide-react";
import { cb, describeFailure, superuserAuth } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
import { cn } from "@/lib/utils";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
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
import { Checkbox } from "@/components/ui/checkbox";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "@/components/ui/context-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Lightbox } from "@/components/ui/lightbox";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { SettingsPage } from "@/components/settings/settings-form";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

/** As `crate::routes::file_manager::list` returns it: `folders` are full
 * prefixes (each already ending in `/`) ready to pass straight back as
 * the next `prefix`, `files` are the leaves sitting directly under the
 * requested prefix — one level, not a recursive walk. */
type ListResponse = { folders: string[]; files: Array<{ key: string; size: number; lastModified: string }> };

/** The server writes its datetime form (a space, not a `T`),
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
  if (superuserAuth.token) headers.authorization = `Bearer ${superuserAuth.token}`;
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
  await cb.send<void>("/api/storage/objects", { method: "POST", body: form });
}

async function deleteObject(key: string): Promise<void> {
  await cb.send<void>("/api/storage/objects", { method: "DELETE", query: { key } });
}

/** Every object under `prefix`, walking into subfolders — `list` itself
 * only returns one level, so a folder rename (which has to move every
 * object beneath it) recurses over it client-side. Cheap enough for an
 * operator tool; this is not a hot path. */
async function listAllKeysUnder(prefix: string): Promise<string[]> {
  const page = await cb.send<ListResponse>("/api/storage/objects", { method: "GET", query: { prefix } });
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

type FileEntry = ListResponse["files"][number];
type Target = { type: "file" | "folder"; key: string };

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|avif|svg|bmp|ico)$/i;
/** Largest image a grid tile fetches for its thumbnail — bigger ones show an
 * icon and are only fetched when opened in the preview. */
const THUMB_MAX_BYTES = 2 * 1024 * 1024;
const VIEW_STORAGE_KEY = "cratebase.files.view";

function isImage(key: string): boolean {
  return IMAGE_EXT.test(key);
}

function FileTypeIcon({ fileKey, className }: { fileKey: string; className?: string }) {
  if (isImage(fileKey)) return <FileImage aria-hidden className={className} />;
  if (/\.(mp4|mov|webm|mkv|avi)$/i.test(fileKey)) return <FileVideo aria-hidden className={className} />;
  if (/\.(mp3|wav|ogg|flac|m4a)$/i.test(fileKey)) return <FileAudio aria-hidden className={className} />;
  if (/\.(zip|tar|gz|tgz|7z|rar)$/i.test(fileKey)) return <FileArchive aria-hidden className={className} />;
  if (/\.(txt|md|json|csv|log|pdf|xml|ya?ml|html?)$/i.test(fileKey)) return <FileText aria-hidden className={className} />;
  return <FileIcon aria-hidden className={className} />;
}

function readView(): "list" | "grid" {
  try {
    return localStorage.getItem(VIEW_STORAGE_KEY) === "grid" ? "grid" : "list";
  } catch {
    return "list";
  }
}

function toastFailure(error: unknown) {
  const failure = describeFailure(error);
  toast.error(failure.title, { description: failure.detail || undefined });
}

/** One object's bytes as a data URL, cached briefly so a thumbnail and the
 * preview that follows it share one download. A data URL (rather than an
 * object URL) has nothing to revoke when the tile unmounts. */
function useObjectUrl(file: FileEntry, enabled: boolean) {
  const { data: url, isError } = useQuery({
    queryKey: ["storage-blob", file.key, file.lastModified],
    queryFn: async () => {
      const blob = await fetchObjectBytes(file.key);
      return new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result));
        reader.onerror = () => reject(reader.error);
        reader.readAsDataURL(blob);
      });
    },
    enabled,
    staleTime: Infinity,
    gcTime: 60_000,
    retry: false,
  });
  return { url: url ?? null, isError };
}

/** Grid-tile thumbnail: fetched only once the tile scrolls into view. */
function Thumb({ file }: { file: FileEntry }) {
  const ref = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const observer = new IntersectionObserver(([entry]) => {
      if (entry?.isIntersecting) {
        setVisible(true);
        observer.disconnect();
      }
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const eligible = isImage(file.key) && file.size <= THUMB_MAX_BYTES;
  const { url, isError } = useObjectUrl(file, visible && eligible);
  return (
    <div ref={ref} className="grid size-full place-items-center bg-surface-sunken">
      {url ? (
        <img src={url} alt="" className="size-full object-cover" draggable={false} />
      ) : eligible && !isError ? (
        <Skeleton className="size-full rounded-none" />
      ) : (
        <FileTypeIcon fileKey={file.key} className="size-8 text-muted-foreground" />
      )}
    </div>
  );
}

/** Full-size image preview in the shared lightbox. */
function FilePreview({ file, onClose }: { file: FileEntry; onClose: () => void }) {
  const { url, isError } = useObjectUrl(file, true);
  useEffect(() => {
    if (isError) {
      toast.error("Couldn't load the preview", { description: baseName(file.key) });
      onClose();
    }
  }, [isError, file.key, onClose]);
  return (
    <>
      <Lightbox
        open={url !== null}
        onClose={onClose}
        src={url ?? ""}
        alt={baseName(file.key)}
        caption={`${baseName(file.key)} · ${formatBytes(file.size)}`}
      />
      {url === null && !isError ? (
        <div
          role="status"
          className="animate-rise fixed bottom-6 left-1/2 z-toast flex -translate-x-1/2 items-center gap-2 rounded-lg border border-border bg-popover px-3 py-2 text-sm shadow-e3"
        >
          <Spinner />
          Loading preview…
          <Button variant="ghost" size="xs" onClick={onClose}>
            Cancel
          </Button>
        </div>
      ) : null}
    </>
  );
}

/** Raw bucket/file browser over the storage backend configured in
 * Application → Storage (`settings.s3`), independent of any record — see
 * `crate::routes::file_manager`'s doc comment for why this exists next to
 * the per-record file API and what "delete" does and doesn't clean up. */
export function FileManagerPage() {
  const queryClient = useQueryClient();
  const [prefix, setPrefix] = useState("");
  const [filter, setFilter] = useState("");
  const [view, setView] = useState<"list" | "grid">(readView);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [deleting, setDeleting] = useState<Target[] | null>(null);
  const [nameAction, setNameAction] = useState<NameAction | null>(null);
  const [previewing, setPreviewing] = useState<FileEntry | null>(null);
  const [dragging, setDragging] = useState(false);
  const dragDepth = useRef(0);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [downloadingKey, setDownloadingKey] = useState<string | null>(null);

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["storage-objects", prefix],
    queryFn: () => cb.send<ListResponse>("/api/storage/objects", { method: "GET", query: { prefix } }),
  });

  function invalidate() {
    return queryClient.invalidateQueries({ queryKey: ["storage-objects"] });
  }

  function navigateTo(next: string) {
    setPrefix(next);
    setFilter("");
    setSelected(new Set());
  }

  function changeView(next: string) {
    if (next !== "list" && next !== "grid") return;
    setView(next);
    try {
      localStorage.setItem(VIEW_STORAGE_KEY, next);
    } catch {
      /* private mode: the choice just doesn't persist */
    }
  }

  const upload = useMutation({
    mutationFn: async (files: File[]) => {
      for (const file of files) await putObject(`${prefix}${file.name}`, file, file.name);
      return files.length;
    },
    onSuccess: async (count) => {
      await invalidate();
      toast.success(count === 1 ? "File uploaded" : `${count} files uploaded`);
    },
    onError: toastFailure,
  });

  const mkdir = useMutation({
    mutationFn: (name: string) => putObject(`${prefix}${name}/.keep`, new Blob([]), ".keep"),
    onSuccess: async () => {
      await invalidate();
      toast.success("Folder created");
      setNameAction(null);
    },
    onError: toastFailure,
  });

  // A file rename downloads the bytes and re-uploads them under the new
  // key; a folder rename does the same for every object underneath it,
  // then clears out every old key once every new one has landed — see
  // `listAllKeysUnder` and `putObject`'s doc comments for why there's no
  // cheaper path.
  const rename = useMutation({
    mutationFn: async ({ target, newName }: { target: Target; newName: string }) => {
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
      setSelected(new Set());
      setNameAction(null);
    },
    onError: toastFailure,
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
    onError: toastFailure,
  });

  const remove = useMutation({
    mutationFn: async (targets: Target[]) => {
      for (const target of targets) {
        if (target.type === "file") {
          await deleteObject(target.key);
        } else {
          for (const key of await listAllKeysUnder(target.key)) await deleteObject(key);
        }
      }
      return targets.length;
    },
    onSuccess: async (count) => {
      await invalidate();
      setSelected(new Set());
      toast.success(count === 1 ? "Deleted" : `Deleted ${count} items`);
    },
    onError: async (error) => {
      // A bulk delete may have removed some items before failing.
      await invalidate();
      toastFailure(error);
    },
  });

  async function handleDownload(key: string) {
    setDownloadingKey(key);
    try {
      await downloadObject(key);
    } catch (error) {
      toastFailure(error);
    } finally {
      setDownloadingKey(null);
    }
  }

  async function copyKey(key: string) {
    try {
      await navigator.clipboard.writeText(key);
      toast.success("Path copied", { description: key });
    } catch {
      toast.error("Couldn't copy to the clipboard");
    }
  }

  function startUpload(files: File[]) {
    if (files.length > 0 && !upload.isPending) upload.mutate(files);
  }

  function handleFilesSelected(event: React.ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.target.files ?? []);
    event.target.value = "";
    startUpload(files);
  }

  const hasFiles = (event: React.DragEvent) => Array.from(event.dataTransfer.types).includes("Files");

  const dropHandlers = {
    onDragEnter(event: React.DragEvent) {
      if (!hasFiles(event)) return;
      event.preventDefault();
      dragDepth.current += 1;
      setDragging(true);
    },
    onDragOver(event: React.DragEvent) {
      if (!hasFiles(event)) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
    },
    onDragLeave(event: React.DragEvent) {
      if (!hasFiles(event)) return;
      dragDepth.current = Math.max(0, dragDepth.current - 1);
      if (dragDepth.current === 0) setDragging(false);
    },
    onDrop(event: React.DragEvent) {
      if (!hasFiles(event)) return;
      event.preventDefault();
      dragDepth.current = 0;
      setDragging(false);
      startUpload(Array.from(event.dataTransfer.files));
    },
  };

  // Breadcrumb entries: "Bucket root" plus one crumb per path segment,
  // each linking back to the prefix it represents.
  const segments = prefix.split("/").filter(Boolean);
  const crumbs = segments.map((segment, i) => ({
    label: segment,
    prefix: `${segments.slice(0, i + 1).join("/")}/`,
  }));

  const allFolders = data?.folders ?? [];
  const allFiles = data?.files ?? [];
  const needle = filter.trim().toLowerCase();
  const folders = needle ? allFolders.filter((f) => baseName(f).toLowerCase().includes(needle)) : allFolders;
  const files = needle ? allFiles.filter((f) => baseName(f.key).toLowerCase().includes(needle)) : allFiles;
  const takenNames = new Set([...allFolders.map((f) => baseName(f)), ...allFiles.map((f) => baseName(f.key))]);
  const visibleKeys = [...folders, ...files.map((f) => f.key)];
  const selectedVisible = visibleKeys.filter((key) => selected.has(key));
  const allSelected = visibleKeys.length > 0 && selectedVisible.length === visibleKeys.length;
  const totalSize = allFiles.reduce((sum, file) => sum + file.size, 0);

  function toggleKey(key: string, on: boolean) {
    setSelected((current) => {
      const next = new Set(current);
      if (on) next.add(key);
      else next.delete(key);
      return next;
    });
  }

  function toggleAll(on: boolean) {
    setSelected(on ? new Set(visibleKeys) : new Set());
  }

  function targetFor(key: string): Target {
    return { type: key.endsWith("/") ? "folder" : "file", key };
  }

  const menuItems = (target: Target, file?: FileEntry) => (
    <>
      {file && isImage(file.key) ? (
        <ContextMenuItem onSelect={() => setPreviewing(file)}>
          <FileImage />
          Preview
        </ContextMenuItem>
      ) : null}
      {target.type === "file" ? (
        <>
          <ContextMenuItem onSelect={() => void handleDownload(target.key)}>
            <Download />
            Download
          </ContextMenuItem>
          <ContextMenuItem onSelect={() => duplicate.mutate(target.key)}>
            <Copy />
            Duplicate
          </ContextMenuItem>
        </>
      ) : null}
      <ContextMenuItem onSelect={() => void copyKey(target.key)}>
        <Copy />
        Copy path
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => setNameAction({ kind: "rename", target })}>
        <Pencil />
        Rename
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem variant="destructive" onSelect={() => setDeleting([target])}>
        <Trash2 />
        Delete
      </ContextMenuItem>
    </>
  );

  const iconActions = (file: FileEntry) => (
    <>
      <Button variant="ghost" size="icon-xs" aria-label={`Copy path of ${baseName(file.key)}`} title="Copy path" onClick={() => void copyKey(file.key)}>
        <Copy />
      </Button>
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label={`Download ${baseName(file.key)}`}
        title="Download"
        onClick={() => void handleDownload(file.key)}
        disabled={downloadingKey === file.key}
      >
        {downloadingKey === file.key ? <Spinner /> : <Download />}
      </Button>
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label={`Delete ${baseName(file.key)}`}
        title="Delete"
        className="text-destructive hover:text-destructive"
        onClick={() => setDeleting([{ type: "file", key: file.key }])}
      >
        <Trash2 />
      </Button>
    </>
  );

  const isEmpty = !isLoading && !isError && allFolders.length === 0 && allFiles.length === 0;
  const noMatches = !isLoading && !isError && !isEmpty && visibleKeys.length === 0;
  const uploadButton = (
    <Button size="sm" onClick={() => fileInputRef.current?.click()} disabled={upload.isPending}>
      {upload.isPending ? <Spinner /> : <Upload />}
      {upload.isPending ? `Uploading ${upload.variables?.length ?? ""}…` : "Upload files"}
    </Button>
  );

  const item = settingsItemFor("/settings/file-manager")!;
  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={
        <>
          <Button variant="outline" size="sm" onClick={() => setNameAction({ kind: "new-folder" })}>
            <FolderPlus />
            New folder
          </Button>
          {uploadButton}
        </>
      }
    >
      <input ref={fileInputRef} type="file" multiple className="hidden" onChange={handleFilesSelected} />

      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 rounded-lg border border-border bg-card px-3 py-1.5">
        <Breadcrumb>
          <BreadcrumbList className="gap-1 font-mono text-xs sm:gap-1">
            <BreadcrumbItem>
              {prefix === "" ? (
                <BreadcrumbPage>Bucket root</BreadcrumbPage>
              ) : (
                <BreadcrumbLink asChild>
                  <button type="button" onClick={() => navigateTo("")}>
                    Bucket root
                  </button>
                </BreadcrumbLink>
              )}
            </BreadcrumbItem>
            {crumbs.map((crumb, i) => (
              <Fragment key={crumb.prefix}>
                <BreadcrumbSeparator />
                <BreadcrumbItem>
                  {i === crumbs.length - 1 ? (
                    <BreadcrumbPage>{crumb.label}</BreadcrumbPage>
                  ) : (
                    <BreadcrumbLink asChild>
                      <button type="button" onClick={() => navigateTo(crumb.prefix)}>
                        {crumb.label}
                      </button>
                    </BreadcrumbLink>
                  )}
                </BreadcrumbItem>
              </Fragment>
            ))}
          </BreadcrumbList>
        </Breadcrumb>
        {data ? (
          <span className="font-tabular text-xs text-muted-foreground">
            {allFolders.length} {allFolders.length === 1 ? "folder" : "folders"} · {allFiles.length}{" "}
            {allFiles.length === 1 ? "file" : "files"} · {formatBytes(totalSize)}
          </span>
        ) : null}
      </div>

      <DataTableToolbar
        search={{ value: filter, onChange: setFilter, placeholder: "Filter by name…", label: "Filter files by name" }}
        actions={
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            spacing={0}
            value={view}
            onValueChange={changeView}
            aria-label="Layout"
          >
            <ToggleGroupItem value="list" aria-label="List view" title="List view">
              <List />
            </ToggleGroupItem>
            <ToggleGroupItem value="grid" aria-label="Grid view" title="Grid view">
              <LayoutGrid />
            </ToggleGroupItem>
          </ToggleGroup>
        }
        selection={
          selectedVisible.length > 0
            ? {
                count: selectedVisible.length,
                onClear: () => setSelected(new Set()),
                actions: (
                  <Button variant="destructive" size="sm" onClick={() => setDeleting(selectedVisible.map(targetFor))}>
                    <Trash2 />
                    Delete
                  </Button>
                ),
              }
            : undefined
        }
      />

      <div className="relative" {...dropHandlers}>
        {isLoading ? (
          view === "grid" ? (
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-6">
              {Array.from({ length: 6 }, (_, i) => (
                <Skeleton key={i} className="aspect-[4/3] w-full rounded-lg" />
              ))}
            </div>
          ) : (
            <ListSkeleton rows={6} />
          )
        ) : isError ? (
          <ErrorState title="Couldn't list this location" error={error} onRetry={() => void refetch()} compact />
        ) : isEmpty ? (
          <EmptyState
            icon={<FolderOpen />}
            title={prefix === "" ? "This bucket is empty" : "This folder is empty"}
            description="Drag files here to upload them, or create a folder to organise what comes next."
            action={
              <div className="flex items-center gap-2">
                <Button variant="outline" size="sm" onClick={() => setNameAction({ kind: "new-folder" })}>
                  <FolderPlus />
                  New folder
                </Button>
                {uploadButton}
              </div>
            }
          />
        ) : noMatches ? (
          <EmptyState
            compact
            icon={<FolderOpen />}
            title="Nothing matches that name"
            description={`No file or folder here contains “${filter.trim()}”.`}
            action={
              <Button variant="outline" size="sm" onClick={() => setFilter("")}>
                Clear filter
              </Button>
            }
          />
        ) : view === "list" ? (
          <div className="overflow-hidden rounded-lg border border-border bg-card">
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="w-9">
                    <Checkbox
                      aria-label="Select all"
                      checked={allSelected ? true : selectedVisible.length > 0 ? "indeterminate" : false}
                      onCheckedChange={(checked) => toggleAll(checked === true)}
                    />
                  </TableHead>
                  <TableHead>Name</TableHead>
                  <TableHead className="w-24 text-right">Size</TableHead>
                  <TableHead className="hidden w-44 sm:table-cell">Modified</TableHead>
                  <TableHead className="w-28" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {folders.map((folder) => (
                  <ContextMenu key={folder}>
                    <ContextMenuTrigger asChild>
                      <TableRow className="h-row" data-state={selected.has(folder) ? "selected" : undefined}>
                        <TableCell>
                          <Checkbox
                            aria-label={`Select ${baseName(folder)}`}
                            checked={selected.has(folder)}
                            onCheckedChange={(checked) => toggleKey(folder, checked === true)}
                          />
                        </TableCell>
                        <TableCell>
                          <button
                            type="button"
                            onClick={() => navigateTo(folder)}
                            className="flex items-center gap-2 rounded-sm font-mono text-xs hover:text-brand-text"
                          >
                            <Folder className="size-3.5 text-brand-text" />
                            {baseName(folder)}/
                          </button>
                        </TableCell>
                        <TableCell className="text-right text-xs text-muted-foreground">—</TableCell>
                        <TableCell className="hidden text-xs text-muted-foreground sm:table-cell">—</TableCell>
                        <TableCell />
                      </TableRow>
                    </ContextMenuTrigger>
                    <ContextMenuContent>{menuItems({ type: "folder", key: folder })}</ContextMenuContent>
                  </ContextMenu>
                ))}
                {files.map((file) => {
                  return (
                    <ContextMenu key={file.key}>
                      <ContextMenuTrigger asChild>
                        <TableRow className="h-row" data-state={selected.has(file.key) ? "selected" : undefined}>
                          <TableCell>
                            <Checkbox
                              aria-label={`Select ${baseName(file.key)}`}
                              checked={selected.has(file.key)}
                              onCheckedChange={(checked) => toggleKey(file.key, checked === true)}
                            />
                          </TableCell>
                          <TableCell>
                            {isImage(file.key) ? (
                              <button
                                type="button"
                                onClick={() => setPreviewing(file)}
                                title="Preview"
                                className="flex items-center gap-2 rounded-sm font-mono text-xs hover:text-brand-text"
                              >
                                <FileTypeIcon fileKey={file.key} className="size-3.5 text-muted-foreground" />
                                {baseName(file.key)}
                              </button>
                            ) : (
                              <span className="flex items-center gap-2 font-mono text-xs">
                                <FileTypeIcon fileKey={file.key} className="size-3.5 text-muted-foreground" />
                                {baseName(file.key)}
                              </span>
                            )}
                          </TableCell>
                          <TableCell className="font-tabular text-right text-xs text-muted-foreground">
                            {formatBytes(file.size)}
                          </TableCell>
                          <TableCell className="font-tabular hidden text-xs text-muted-foreground sm:table-cell">
                            {parseServerDate(file.lastModified).toLocaleString()}
                          </TableCell>
                          <TableCell>
                            <div className="flex justify-end gap-0.5">{iconActions(file)}</div>
                          </TableCell>
                        </TableRow>
                      </ContextMenuTrigger>
                      <ContextMenuContent>{menuItems({ type: "file", key: file.key }, file)}</ContextMenuContent>
                    </ContextMenu>
                  );
                })}
              </TableBody>
            </Table>
          </div>
        ) : (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-6">
            {folders.map((folder, i) => (
              <ContextMenu key={folder}>
                <ContextMenuTrigger asChild>
                  <div
                    style={{ "--i": Math.min(i, 8) } as React.CSSProperties}
                    className={cn(
                      "animate-rise group relative flex flex-col overflow-hidden rounded-lg border bg-card transition-colors duration-fast ease-out-strong hover:border-border-strong",
                      selected.has(folder) ? "border-primary" : "border-border",
                    )}
                  >
                    <button
                      type="button"
                      onClick={() => navigateTo(folder)}
                      className="flex aspect-[4/3] w-full flex-col items-center justify-center gap-1 bg-surface-sunken"
                      aria-label={`Open folder ${baseName(folder)}`}
                    >
                      <Folder aria-hidden className="size-9 text-brand-text" />
                    </button>
                    <div className="truncate border-t border-border px-2.5 py-1.5 font-mono text-xs">{baseName(folder)}/</div>
                    <Checkbox
                      className="absolute top-2 left-2 bg-card"
                      aria-label={`Select ${baseName(folder)}`}
                      checked={selected.has(folder)}
                      onCheckedChange={(checked) => toggleKey(folder, checked === true)}
                    />
                  </div>
                </ContextMenuTrigger>
                <ContextMenuContent>{menuItems({ type: "folder", key: folder })}</ContextMenuContent>
              </ContextMenu>
            ))}
            {files.map((file, i) => (
              <ContextMenu key={file.key}>
                <ContextMenuTrigger asChild>
                  <div
                    style={{ "--i": Math.min(i + folders.length, 8) } as React.CSSProperties}
                    className={cn(
                      "animate-rise group relative flex flex-col overflow-hidden rounded-lg border bg-card transition-colors duration-fast ease-out-strong hover:border-border-strong",
                      selected.has(file.key) ? "border-primary" : "border-border",
                    )}
                  >
                    {isImage(file.key) ? (
                      <button
                        type="button"
                        onClick={() => setPreviewing(file)}
                        aria-label={`Preview ${baseName(file.key)}`}
                        className="aspect-[4/3] w-full"
                      >
                        <Thumb file={file} />
                      </button>
                    ) : (
                      <div className="aspect-[4/3] w-full">
                        <Thumb file={file} />
                      </div>
                    )}
                    <div className="flex min-w-0 flex-col border-t border-border px-2.5 py-1.5">
                      <span className="truncate font-mono text-xs" title={baseName(file.key)}>
                        {baseName(file.key)}
                      </span>
                      <span className="font-tabular text-2xs text-muted-foreground">{formatBytes(file.size)}</span>
                    </div>
                    <Checkbox
                      className="absolute top-2 left-2 bg-card"
                      aria-label={`Select ${baseName(file.key)}`}
                      checked={selected.has(file.key)}
                      onCheckedChange={(checked) => toggleKey(file.key, checked === true)}
                    />
                    <div className="absolute top-1.5 right-1.5 flex gap-0.5 rounded-md border border-border bg-card/90 p-0.5 opacity-0 backdrop-blur-sm transition-opacity duration-fast group-focus-within:opacity-100 group-hover:opacity-100 [@media(hover:none)]:opacity-100">
                      {iconActions(file)}
                    </div>
                  </div>
                </ContextMenuTrigger>
                <ContextMenuContent>{menuItems({ type: "file", key: file.key }, file)}</ContextMenuContent>
              </ContextMenu>
            ))}
          </div>
        )}

        {dragging ? (
          <div
            aria-hidden
            className="pointer-events-none absolute inset-0 z-sticky grid place-items-center rounded-lg border-2 border-dashed border-primary bg-background/85 backdrop-blur-xs"
          >
            <div className="flex flex-col items-center gap-1 text-center">
              <Upload className="size-6 text-brand-text" />
              <p className="text-sm font-medium">Drop to upload to {prefix === "" ? "the bucket root" : <span className="font-mono">{prefix}</span>}</p>
              <p className="text-xs text-muted-foreground">Files with the same name are replaced.</p>
            </div>
          </div>
        ) : null}
      </div>

      {!isLoading && !isError && !isEmpty ? (
        <p className="text-xs text-muted-foreground">
          Drag files anywhere on the list to upload. Right-click an item for rename, duplicate and more.
        </p>
      ) : null}

      {previewing ? <FilePreview file={previewing} onClose={() => setPreviewing(null)} /> : null}

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

      {deleting ? (
        <DeleteDialog
          targets={deleting}
          onCancel={() => setDeleting(null)}
          onConfirm={() => {
            const targets = deleting;
            setDeleting(null);
            remove.mutate(targets);
          }}
        />
      ) : null}
    </SettingsPage>
  );
}

/** Deleting one file is a plain confirm; deleting a folder (which takes
 * everything inside it) or several items at once makes the operator type
 * what they are about to destroy. */
function DeleteDialog({
  targets,
  onCancel,
  onConfirm,
}: {
  targets: Target[];
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const [typed, setTyped] = useState("");
  const single = targets.length === 1 ? targets[0]! : null;
  const hasFolder = targets.some((t) => t.type === "folder");
  const phrase = single ? (single.type === "folder" ? baseName(single.key) : null) : "delete";
  const ready = phrase === null || typed.trim() === phrase;
  const noun = single ? (single.type === "folder" ? "folder" : "file") : `${targets.length} items`;

  return (
    <AlertDialog open onOpenChange={(open) => !open && onCancel()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete {single ? `this ${noun}` : noun}?</AlertDialogTitle>
          <AlertDialogDescription asChild>
            <div className="flex flex-col gap-2">
              {single ? (
                <p>
                  <span className="font-mono">{baseName(single.key)}</span>
                  {single.type === "folder" ? " and everything inside it" : ""} will be removed from storage permanently.
                </p>
              ) : (
                <>
                  <p>
                    These will be removed from storage permanently{hasFolder ? ", including everything inside any folder" : ""}:
                  </p>
                  <ul className="max-h-28 overflow-auto rounded-md border border-border bg-surface-sunken px-2.5 py-1.5 font-mono text-xs">
                    {targets.map((t) => (
                      <li key={t.key} className="truncate">
                        {baseName(t.key)}
                        {t.type === "folder" ? "/" : ""}
                      </li>
                    ))}
                  </ul>
                </>
              )}
              <p>
                This does <strong>not</strong> update any record that references a file here — the record will point at a
                now-missing file. This cannot be undone.
              </p>
            </div>
          </AlertDialogDescription>
        </AlertDialogHeader>
        {phrase !== null ? (
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="delete-confirm">
              Type <span className="font-mono font-semibold">{phrase}</span> to confirm
            </Label>
            <Input
              id="delete-confirm"
              autoFocus
              autoComplete="off"
              spellCheck={false}
              value={typed}
              onChange={(e) => setTyped(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && ready) onConfirm();
              }}
            />
          </div>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction variant="destructive" disabled={!ready} onClick={onConfirm}>
            Delete {noun}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
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
