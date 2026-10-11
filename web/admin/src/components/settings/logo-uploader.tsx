import { useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ChevronDown, ImagePlus, RefreshCw, Trash2 } from "lucide-react";
import { cb, describeFailure } from "@/lib/api";
import { cn } from "@/lib/utils";
import { SETTINGS_KEY, type ServerSettings } from "@/hooks/use-settings";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
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
import { TextSetting } from "@/components/settings/settings-form";

const MAX_BYTES = 1024 * 1024;
const ACCEPT = "image/png,image/jpeg,image/webp,image/gif";
const ALLOWED_TYPES = ACCEPT.split(",");
const UPLOADED_PATH = "/api/branding/logo";

/** Whether `logoUrl` is the one `POST /api/settings/logo` writes. */
function isUploadedLogo(logoUrl: string): boolean {
  return logoUrl.trim().split("?")[0].endsWith(UPLOADED_PATH);
}

/** A same-origin URL for previewing an uploaded logo — the stored absolute
 * URL points at `appURL`, which may not be reachable from this browser. */
function previewSrc(logoUrl: string): string {
  const url = logoUrl.trim();
  if (!isUploadedLogo(url)) return url;
  const query = url.includes("?") ? url.slice(url.indexOf("?")) : "";
  return cb.buildURL(`${UPLOADED_PATH}${query}`);
}

function validateFile(file: File): string | null {
  if (!ALLOWED_TYPES.includes(file.type)) {
    return file.type === "image/svg+xml"
      ? "SVG isn't supported — most email clients can't show it. Use a PNG, JPEG, WebP or GIF."
      : "Use a PNG, JPEG, WebP or GIF image.";
  }
  if (file.size > MAX_BYTES) return "That image is larger than 1 MiB. Choose a smaller one.";
  return null;
}

const CHECKER: React.CSSProperties = {
  backgroundImage:
    "conic-gradient(var(--border) 25%, transparent 0 50%, var(--border) 0 75%, transparent 0)",
  backgroundSize: "10px 10px",
};

interface LogoUploaderProps {
  /** Current `meta.logoUrl` (the form draft, or the saved value). */
  value: string;
  /** Called with the new `logoUrl` after an upload/removal (already
   * persisted server-side) or when the URL field is edited (not saved). */
  onChange: (logoUrl: string) => void;
  error?: string;
  /** Show the "Or use an image URL" disclosure. */
  allowUrl?: boolean;
  id?: string;
}

/**
 * Upload (drag-and-drop or file picker), preview, replace and remove the
 * brand logo. Uploading goes straight to `POST /api/settings/logo`, which
 * stores the file and updates `settings.meta.logoUrl` itself — so unlike the
 * rest of the settings form it does not wait for Save. The settings cache is
 * updated and refetched here; callers only mirror the new URL into any draft.
 */
export function LogoUploader({ value, onChange, error, allowUrl = true, id = "logo-uploader" }: LogoUploaderProps) {
  const queryClient = useQueryClient();
  const inputRef = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState<"upload" | "remove" | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const [dragging, setDragging] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [urlOpen, setUrlOpen] = useState(() => value.trim() !== "" && !isUploadedLogo(value));
  const [previewFailed, setPreviewFailed] = useState(false);

  const logo = value.trim();
  const hasLogo = logo !== "" && !error;

  function syncSettings(logoUrl: string) {
    queryClient.setQueryData<ServerSettings>(SETTINGS_KEY, (s) =>
      s ? { ...s, meta: { ...s.meta, logoUrl } } : s,
    );
    void queryClient.invalidateQueries({ queryKey: SETTINGS_KEY });
  }

  async function upload(file: File | undefined) {
    if (!file || busy) return;
    const problem = validateFile(file);
    if (problem) {
      setFailure(problem);
      setStatus(problem);
      return;
    }
    setFailure(null);
    setBusy("upload");
    setStatus("Uploading logo…");
    try {
      const form = new FormData();
      form.append("file", file, file.name);
      const { logoUrl } = await cb.send<{ logoUrl: string }>("/api/settings/logo", { method: "POST", body: form });
      setPreviewFailed(false);
      onChange(logoUrl);
      syncSettings(logoUrl);
      setStatus("Logo uploaded.");
    } catch (e) {
      const f = describeFailure(e);
      const message = f.serverMessage || f.detail || f.title;
      setFailure(message);
      setStatus(`Upload failed: ${message}`);
    } finally {
      setBusy(null);
      if (inputRef.current) inputRef.current.value = "";
    }
  }

  async function remove() {
    setFailure(null);
    if (!isUploadedLogo(logo)) {
      // An external URL lives only in the form draft.
      onChange("");
      setStatus("Logo removed.");
      return;
    }
    setBusy("remove");
    try {
      await cb.send<void>("/api/settings/logo", { method: "DELETE" });
      onChange("");
      syncSettings("");
      setStatus("Logo removed.");
    } catch (e) {
      const f = describeFailure(e);
      const message = f.serverMessage || f.detail || f.title;
      setFailure(message);
      setStatus(`Couldn't remove the logo: ${message}`);
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="flex flex-col gap-2">
      <div
        onDragOver={(e) => {
          e.preventDefault();
          if (!dragging) setDragging(true);
        }}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
        }}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          void upload(e.dataTransfer.files[0]);
        }}
        className={cn(
          "flex flex-wrap items-center gap-3 rounded-lg border border-dashed p-3 transition-colors duration-fast",
          dragging ? "border-primary bg-brand-soft" : "border-border-strong bg-surface-sunken",
          (failure || error) && "border-destructive/50",
        )}
      >
        {hasLogo ? (
          <div className="flex shrink-0 items-center gap-2">
            <div
              className="grid size-16 place-items-center overflow-hidden rounded-md border border-border bg-background"
              style={CHECKER}
            >
              <img
                key={logo}
                src={previewSrc(logo)}
                alt="Logo preview"
                className="max-h-full max-w-full object-contain p-1"
                onError={() => setPreviewFailed(true)}
                onLoad={() => setPreviewFailed(false)}
              />
            </div>
            <div className="grid size-16 place-items-center overflow-hidden rounded-md border border-border bg-foreground">
              <img
                src={previewSrc(logo)}
                alt=""
                aria-hidden
                className="max-h-full max-w-full object-contain p-1"
              />
            </div>
          </div>
        ) : (
          <div className="grid size-10 shrink-0 place-items-center rounded-md border border-border bg-background text-muted-foreground">
            <ImagePlus className="size-4" />
          </div>
        )}

        <div className="flex min-w-0 flex-1 flex-col gap-1.5">
          <p className="text-sm text-muted-foreground">
            {busy === "upload"
              ? "Uploading…"
              : hasLogo
                ? "Drop a new image here to replace it."
                : "Drag an image here, or choose a file."}
          </p>
          <p className="text-xs text-muted-foreground">PNG, JPEG, WebP or GIF · up to 1 MiB</p>
          <div className="flex flex-wrap items-center gap-1.5">
            <input
              ref={inputRef}
              id={id}
              type="file"
              accept={ACCEPT}
              className="sr-only"
              tabIndex={-1}
              onChange={(e) => void upload(e.target.files?.[0])}
            />
            <Button
              type="button"
              size="sm"
              variant={hasLogo ? "outline" : "default"}
              disabled={busy !== null}
              onClick={() => inputRef.current?.click()}
            >
              {busy === "upload" ? <Spinner /> : hasLogo ? <RefreshCw /> : <ImagePlus />}
              {hasLogo ? "Replace" : "Choose file"}
            </Button>
            {hasLogo ? (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="text-muted-foreground hover:text-destructive"
                disabled={busy !== null}
                onClick={() => (isUploadedLogo(logo) ? setConfirmRemove(true) : void remove())}
              >
                {busy === "remove" ? <Spinner /> : <Trash2 />}
                Remove
              </Button>
            ) : null}
          </div>
        </div>
      </div>

      {failure ? (
        <p role="alert" className="text-xs text-destructive">
          {failure}
        </p>
      ) : null}
      {hasLogo && previewFailed ? (
        <p className="text-xs text-warning">
          The preview didn't load. Check that the URL is public and points at an image.
        </p>
      ) : null}
      <span role="status" aria-live="polite" className="sr-only">
        {status}
      </span>

      {allowUrl ? (
        <Collapsible open={urlOpen} onOpenChange={setUrlOpen}>
          <CollapsibleTrigger asChild>
            <button
              type="button"
              className="flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
            >
              <ChevronDown className={cn("size-3 transition-transform", urlOpen && "rotate-180")} />
              Or use an image URL
            </button>
          </CollapsibleTrigger>
          <CollapsibleContent className="pt-1.5">
            <TextSetting
              id={`${id}-url`}
              value={value}
              onChange={(next) => {
                setPreviewFailed(false);
                onChange(next);
              }}
              placeholder="https://example.com/logo.png"
              mono
              invalid={!!error}
            />
          </CollapsibleContent>
        </Collapsible>
      ) : null}

      <AlertDialog open={confirmRemove} onOpenChange={setConfirmRemove}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove the logo?</AlertDialogTitle>
            <AlertDialogDescription>
              The uploaded image is deleted and system emails go back to a text wordmark of the application name.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setConfirmRemove(false);
                void remove();
              }}
            >
              Remove logo
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
