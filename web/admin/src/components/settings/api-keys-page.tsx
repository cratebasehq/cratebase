import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Check, Copy, KeyRound, Plus, Trash2 } from "lucide-react";
import { cb, describeFailure } from "@/lib/api";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { Button } from "@/components/ui/button";
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
import { Switch } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

/** An `_api_keys` record as the generic Records API returns it — never
 * `key` itself (it's `hidden` and stores a one-way hash anyway; see
 * `crates/server/src/api_keys.rs`). */
interface ApiKeyRecord {
  id: string;
  name: string;
  prefix: string;
  enabled: boolean;
  lastUsedAt: string;
  created: string;
}

/** `POST /api/api-keys`'s response — the one and only time the raw key
 * is ever sent to a client. */
interface MintedApiKey {
  id: string;
  name: string;
  prefix: string;
  key: string;
}

/**
 * CRUD over `_api_keys` — bearer credentials (`cb_...`) a superuser mints
 * for scripts, CI jobs, or MCP clients (see the MCP settings page), each
 * resolving to a superuser identity server-side
 * (`crates/server/src/api_keys.rs`). Minting goes through the dedicated
 * `POST /api/api-keys` endpoint, the only place the raw key is ever
 * returned; everything else here is the ordinary generic Records API.
 */
export function ApiKeysPage() {
  const queryClient = useQueryClient();
  const [creating, setCreating] = useState(false);
  const [minted, setMinted] = useState<MintedApiKey | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ApiKeyRecord | null>(null);

  const { data, isLoading, error } = useQuery({
    queryKey: ["api-keys"],
    queryFn: () => cb.collection("_api_keys").getFullList<ApiKeyRecord>({ sort: "-created", requestKey: null }),
  });

  function invalidate() {
    return queryClient.invalidateQueries({ queryKey: ["api-keys"] });
  }

  const toggle = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      cb.collection("_api_keys").update(id, { enabled }),
    onSuccess: () => void invalidate(),
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection("_api_keys").delete(id),
    onSuccess: () => {
      toast.success("API key revoked");
      void invalidate();
      setDeleteTarget(null);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const keys = data ?? [];

  return (
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-3 p-page">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="flex flex-col gap-2">
          <h2 className="text-sm font-medium">API keys</h2>
          <p className="max-w-measure text-sm text-muted-foreground">
            Bearer credentials for scripts, CI jobs, and MCP clients. Each key resolves to a superuser identity —
            treat it like a password. The full key is shown once, right after creation, and never again.
          </p>
        </div>
        <Button size="sm" className="gap-1.5 shrink-0" onClick={() => setCreating(true)}>
          <Plus className="size-3.5" />
          New API key
        </Button>
      </div>

      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <KeyRound />
            </EmptyMedia>
            <EmptyTitle>Couldn't load API keys</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : isLoading ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 2 }, (_, i) => (
            <Skeleton key={i} className="h-row w-full" />
          ))}
        </div>
      ) : keys.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <KeyRound />
            </EmptyMedia>
            <EmptyTitle>No API keys yet</EmptyTitle>
            <EmptyDescription>Mint one to authenticate a script, CI job, or MCP client.</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Key</TableHead>
              <TableHead>Last used</TableHead>
              <TableHead className="w-[90px]">Enabled</TableHead>
              <TableHead className="w-[60px]" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {keys.map((key) => (
              <TableRow key={key.id}>
                <TableCell className="font-medium">{key.name || <span className="text-muted-foreground">Untitled</span>}</TableCell>
                <TableCell className="font-mono text-xs text-muted-foreground">cb_{key.prefix}…</TableCell>
                <TableCell className="text-xs text-muted-foreground">
                  {key.lastUsedAt ? new Date(key.lastUsedAt.replace(" ", "T")).toLocaleString() : "Never"}
                </TableCell>
                <TableCell>
                  <Switch
                    checked={key.enabled}
                    aria-label={key.enabled ? `Disable ${key.name}` : `Enable ${key.name}`}
                    onCheckedChange={(checked) => toggle.mutate({ id: key.id, enabled: checked })}
                  />
                </TableCell>
                <TableCell>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Delete ${key.name}`}
                    onClick={() => setDeleteTarget(key)}
                  >
                    <Trash2 className="size-3.5 text-destructive" />
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      {creating ? (
        <CreateApiKeyDialog
          onOpenChange={(open) => !open && setCreating(false)}
          onMinted={(key) => {
            setCreating(false);
            setMinted(key);
            void invalidate();
          }}
        />
      ) : null}

      {minted ? <RevealKeyDialog minted={minted} onOpenChange={(open) => !open && setMinted(null)} /> : null}

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Revoke "{deleteTarget?.name}"?</DialogTitle>
            <DialogDescription>
              Any script or client using this key loses access immediately. There is no undo — a replacement needs a
              new key.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={remove.isPending}
              onClick={() => deleteTarget && remove.mutate(deleteTarget.id)}
            >
              {remove.isPending ? <Spinner className="size-3.5" /> : null}
              Revoke
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function CreateApiKeyDialog({
  onOpenChange,
  onMinted,
}: {
  onOpenChange: (open: boolean) => void;
  onMinted: (key: MintedApiKey) => void;
}) {
  const [name, setName] = useState("");

  const create = useMutation({
    mutationFn: () => cb.send<MintedApiKey>("/api/api-keys", { method: "POST", body: { name } }),
    onSuccess: onMinted,
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>New API key</DialogTitle>
          <DialogDescription>Names a key for your own reference — it plays no part in authentication.</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="api-key-name">Name</Label>
          <Input
            id="api-key-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Deploy script"
            autoFocus
          />
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={create.isPending} onClick={() => create.mutate()}>
            {create.isPending ? <Spinner className="size-3.5" /> : null}
            Create key
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Shows the raw key exactly once — the server never sends it again
 * (`_api_keys.key` is `hidden` and stores only a hash). Dismissing is
 * the only way out, same one-time-secret convention as a webhook's
 * signing secret being write-only after creation. */
function RevealKeyDialog({ minted, onOpenChange }: { minted: MintedApiKey; onOpenChange: (open: boolean) => void }) {
  const { copy, status } = useCopyToClipboard();

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Copy this key now</DialogTitle>
          <DialogDescription>
            This is the only time "{minted.name || "this key"}" is shown in full. Once you close this dialog,
            Cratebase keeps only a hash of it — there is no way to retrieve it again.
          </DialogDescription>
        </DialogHeader>

        <div className="relative">
          <pre className="overflow-x-auto rounded-lg border border-border bg-surface-sunken px-3 py-2.5 font-mono text-xs leading-relaxed text-foreground">
            {minted.key}
          </pre>
          <button
            type="button"
            onClick={() => void copy(minted.key)}
            aria-label={status === "copied" ? "Copied" : "Copy key"}
            className="absolute right-1.5 top-1.5 grid size-control-xs place-items-center rounded border border-border bg-background text-muted-foreground transition-colors hover:text-foreground"
          >
            {status === "copied" ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
          </button>
        </div>

        <DialogFooter>
          <Button onClick={() => onOpenChange(false)}>I've copied it</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
