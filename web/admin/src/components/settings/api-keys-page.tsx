import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Check, Copy, KeyRound, Plus, Trash2 } from "lucide-react";
import { cb, describeFailure } from "@/lib/api";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { useCollections } from "@/hooks/use-collections";
import { settingsItemFor } from "@/lib/settings-nav";
import { RelationPicker } from "@/components/records/relation-picker";
import { Badge } from "@/components/ui/badge";
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
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { SettingsPage } from "@/components/settings/settings-form";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

/** An `_api_keys` record as the generic Records API returns it — never
 * `key` itself (it's `hidden` and stores a one-way hash anyway; see
 * `crates/server/src/api_keys.rs`). `actsAsCollection`/`actsAsRecord` are
 * both `""` for an unscoped (superuser) key — see the same file's module
 * doc's "Scoping" section. */
interface ApiKeyRecord {
  id: string;
  name: string;
  prefix: string;
  enabled: boolean;
  lastUsedAt: string;
  created: string;
  actsAsCollection: string;
  actsAsRecord: string;
}

/** `POST /api/api-keys`'s response — the one and only time the raw key
 * is ever sent to a client. */
interface MintedApiKey {
  id: string;
  name: string;
  prefix: string;
  actsAsCollection: string;
  actsAsRecord: string;
  key: string;
}

/**
 * CRUD over `_api_keys` — bearer credentials (`cb_...`) a superuser mints
 * for scripts, CI jobs, or MCP clients (see the MCP settings page). Each
 * resolves to a superuser identity by default, or, when scoped at mint
 * time, to a real record's own identity — see `crates/server/src/api_keys.rs`'s
 * module doc's "Scoping" section. Minting goes through the dedicated
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

  const item = settingsItemFor("/settings/api-keys")!;
  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={
        <Button size="sm" className="gap-1.5" onClick={() => setCreating(true)}>
          <Plus className="size-3.5" />
          New API key
        </Button>
      }
    >

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
              <TableHead>Access</TableHead>
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
                <TableCell>
                  {key.actsAsCollection ? (
                    <Badge variant="outline" className="font-normal">
                      {key.actsAsCollection}
                    </Badge>
                  ) : (
                    <Badge variant="secondary" className="font-normal">
                      Superuser
                    </Badge>
                  )}
                </TableCell>
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
    </SettingsPage>
  );
}

/** `"superuser"` mints unscoped root, the only shape this dialog offered
 * before scoping existed and still the default. `"record"` picks a real
 * auth-collection record for the key to act as — see
 * `crates/server/src/api_keys.rs`'s module doc's "Scoping" section for
 * what that changes server-side. */
type ActsAsMode = "superuser" | "record";

function CreateApiKeyDialog({
  onOpenChange,
  onMinted,
}: {
  onOpenChange: (open: boolean) => void;
  onMinted: (key: MintedApiKey) => void;
}) {
  const [name, setName] = useState("");
  const [mode, setMode] = useState<ActsAsMode>("superuser");
  const [collectionId, setCollectionId] = useState<string | undefined>(undefined);
  const [recordId, setRecordId] = useState<string | undefined>(undefined);

  const { data: collections } = useCollections();
  const authCollections = (collections ?? []).filter((c) => c.type === "auth");
  const targetCollection = authCollections.find((c) => c.id === collectionId);

  const create = useMutation({
    mutationFn: () =>
      cb.send<MintedApiKey>("/api/api-keys", {
        method: "POST",
        body:
          mode === "record" && targetCollection && recordId
            ? { name, actsAsCollection: targetCollection.name, actsAsRecord: recordId }
            : { name },
      }),
    onSuccess: onMinted,
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const canCreate = mode === "superuser" || (collectionId !== undefined && recordId !== undefined);

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

        <div className="flex flex-col gap-1.5">
          <Label>Acts as</Label>
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            spacing={0}
            aria-label="Acts as"
            value={mode}
            // A segmented control always has exactly one option picked —
            // Radix reports "" when the pressed item is toggled off.
            onValueChange={(next) => {
              if (next) setMode(next as ActsAsMode);
            }}
          >
            <ToggleGroupItem value="superuser">Superuser (full access)</ToggleGroupItem>
            <ToggleGroupItem value="record">A specific record</ToggleGroupItem>
          </ToggleGroup>
          <p className="text-xs text-muted-foreground">
            {mode === "superuser"
              ? "Unrestricted root, bypassing every collection rule — never hand this to untrusted code."
              : "The key gets exactly the rule-gated access that record has, nothing more — the same access it would have logging in normally."}
          </p>
        </div>

        {mode === "record" ? (
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="api-key-acts-as-collection">Collection</Label>
            <Select
              value={collectionId}
              onValueChange={(next) => {
                setCollectionId(next);
                setRecordId(undefined);
              }}
            >
              <SelectTrigger id="api-key-acts-as-collection" className="w-full">
                <SelectValue placeholder="Pick an auth collection" />
              </SelectTrigger>
              <SelectContent>
                {authCollections.map((c) => (
                  <SelectItem key={c.id} value={c.id}>
                    {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            {collectionId ? (
              <RelationPicker
                collectionId={collectionId}
                fieldName="Record"
                value={recordId ? [recordId] : []}
                onChange={(ids) => setRecordId(ids[0])}
                multiple={false}
                maxSelect={1}
              />
            ) : null}
          </div>
        ) : null}

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={create.isPending || !canCreate} onClick={() => create.mutate()}>
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
