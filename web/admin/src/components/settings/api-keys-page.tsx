import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Check, Copy, KeyRound, Plus, ShieldAlert, Trash2 } from "lucide-react";
import { cb, describeFailure, parseServerDate } from "@/lib/api";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { useCollections } from "@/hooks/use-collections";
import { settingsItemFor } from "@/lib/settings-nav";
import { cn } from "@/lib/utils";
import { RelationPicker } from "@/components/records/relation-picker";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { StatusBadge } from "@/components/layout/stat-card";
import { Button } from "@/components/ui/button";
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
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { SettingsPage } from "@/components/settings/settings-form";
import {
  DetailBody,
  DetailCallout,
  DetailChip,
  DetailList,
  DetailRow,
  DetailSection,
  DetailSheet,
  DetailSheetHeader,
  DetailTabPanel,
  DetailTabs,
} from "@/components/layout/detail-sheet";
import { CodeBlock } from "@/components/settings/detail-json";
import { TimeAgo } from "@/components/settings/time-ago";
import { TypedConfirmDialog } from "@/components/settings/typed-confirm-dialog";
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

/** Table cell classes: dense rows, small muted header. */
const HEAD = "h-control-md text-xs font-medium text-muted-foreground";
const CELL = "py-1.5";

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
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const { data, isPending, error, refetch } = useQuery({
    queryKey: ["api-keys"],
    queryFn: async () =>
      // `_api_keys` is a system collection with no `Schema` entry — the
      // server's own shape (`crates/core/src/collection.rs`) is trusted here.
      (await cb.collection("_api_keys").fullList({ sort: "-created" })) as unknown as ApiKeyRecord[],
  });

  function invalidate() {
    return queryClient.invalidateQueries({ queryKey: ["api-keys"] });
  }

  const toggle = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean; name: string }) =>
      cb.collection("_api_keys").update(id, { enabled }),
    onSuccess: (_result, vars) => {
      toast.success(vars.enabled ? `Enabled “${vars.name}”` : `Disabled “${vars.name}”`);
      void invalidate();
    },
    onError: (failure, vars) => {
      const described = describeFailure(failure);
      toast.error(`Couldn't ${vars.enabled ? "enable" : "disable"} “${vars.name}”`, {
        description: described.serverMessage || described.detail,
      });
    },
  });

  const remove = useMutation({
    mutationFn: (key: ApiKeyRecord) => cb.collection("_api_keys").delete(key.id),
    onSuccess: (_result, key) => {
      toast.success(`Revoked “${keyLabel(key)}”`);
      void invalidate();
      setDeleteTarget(null);
    },
    onError: (failure, key) => {
      const described = describeFailure(failure);
      toast.error(`Couldn't revoke “${keyLabel(key)}”`, { description: described.serverMessage || described.detail });
    },
  });

  const keys = data ?? [];
  const showCreateAction = !isPending && !error && keys.length > 0;
  const selectedIndex = keys.findIndex((k) => k.id === selectedId);
  const selected = selectedIndex >= 0 ? keys[selectedIndex] : null;

  const item = settingsItemFor("/settings/api-keys")!;
  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={
        showCreateAction ? (
          <Button size="sm" onClick={() => setCreating(true)}>
            <Plus />
            New API key
          </Button>
        ) : undefined
      }
    >
      {error ? (
        <ErrorState compact title="Couldn't load API keys" error={describeFailure(error).detail} onRetry={() => void refetch()} />
      ) : isPending ? (
        <ListSkeleton rows={3} />
      ) : keys.length === 0 ? (
        <EmptyState
          compact
          icon={<KeyRound />}
          title="No API keys yet"
          description="Create a key to authenticate a script, CI job, or MCP client with an Authorization: Bearer header."
          action={
            <Button size="sm" onClick={() => setCreating(true)}>
              <Plus />
              New API key
            </Button>
          }
        />
      ) : (
        <div className="overflow-hidden rounded-lg border border-border bg-card">
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className={cn(HEAD, "pl-3")}>Name</TableHead>
                <TableHead className={HEAD}>Key</TableHead>
                <TableHead className={HEAD}>Access</TableHead>
                <TableHead className={HEAD}>Last used</TableHead>
                <TableHead className={cn(HEAD, "w-[90px]")}>Enabled</TableHead>
                <TableHead className={cn(HEAD, "w-12 pr-3")}>
                  <span className="sr-only">Actions</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {keys.map((key) => {
                const label = keyLabel(key);
                return (
                  <TableRow
                    key={key.id}
                    tabIndex={0}
                    role="button"
                    aria-label={`${label}, open details`}
                    data-state={key.id === selectedId ? "selected" : undefined}
                    onClick={() => setSelectedId(key.id)}
                    onKeyDown={(event) => {
                      if (event.target === event.currentTarget && (event.key === "Enter" || event.key === " ")) {
                        event.preventDefault();
                        setSelectedId(key.id);
                      }
                    }}
                    className={cn("cursor-pointer", !key.enabled && "text-muted-foreground")}
                  >
                    <TableCell className={cn(CELL, "pl-3 font-medium")}>
                      {key.name || <span className="text-muted-foreground">Untitled</span>}
                    </TableCell>
                    <TableCell className={cn(CELL, "font-mono text-xs text-muted-foreground")}>cb_{key.prefix}…</TableCell>
                    <TableCell className={CELL}>
                      {key.actsAsCollection ? (
                        <span className="inline-flex items-center gap-2">
                          <StatusBadge tone="info">Acts as record</StatusBadge>
                          <span
                            className="max-w-48 truncate font-mono text-xs text-muted-foreground"
                            title={`${key.actsAsCollection}/${key.actsAsRecord}`}
                          >
                            {key.actsAsCollection}/{key.actsAsRecord}
                          </span>
                        </span>
                      ) : (
                        <StatusBadge tone="warning">Superuser</StatusBadge>
                      )}
                    </TableCell>
                    <TableCell className={cn(CELL, "text-xs text-muted-foreground")}>
                      <TimeAgo value={key.lastUsedAt} fallback="Never" />
                    </TableCell>
                    <TableCell className={CELL} onClick={(e) => e.stopPropagation()}>
                      <Switch
                        checked={key.enabled}
                        disabled={toggle.isPending && toggle.variables?.id === key.id}
                        aria-label={key.enabled ? `Disable ${label}` : `Enable ${label}`}
                        onCheckedChange={(checked) => toggle.mutate({ id: key.id, enabled: checked, name: label })}
                      />
                    </TableCell>
                    <TableCell className={cn(CELL, "pr-3 text-right")} onClick={(e) => e.stopPropagation()}>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Revoke ${label}`}
                        title="Revoke key"
                        className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                        onClick={() => setDeleteTarget(key)}
                      >
                        <Trash2 />
                      </Button>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
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

      {minted ? <RevealKeyDialog minted={minted} onDone={() => setMinted(null)} /> : null}

      <TypedConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => !open && !remove.isPending && setDeleteTarget(null)}
        title={`Revoke “${deleteTarget ? keyLabel(deleteTarget) : ""}”?`}
        description="Any script or client using this key loses access immediately. There is no undo — a replacement needs a new key."
        confirmValue={deleteTarget ? keyLabel(deleteTarget) : ""}
        confirmLabel="Revoke key"
        pendingLabel="Revoking…"
        pending={remove.isPending}
        onConfirm={() => deleteTarget && remove.mutate(deleteTarget)}
      />

      <DetailSheet open={selected !== null} onOpenChange={(open) => !open && setSelectedId(null)}>
        {selected ? (
          <ApiKeyDetail
            apiKey={selected}
            index={selectedIndex}
            total={keys.length}
            onStep={(delta) => {
              const next = keys[selectedIndex + delta];
              if (next) setSelectedId(next.id);
            }}
            toggling={toggle.isPending && toggle.variables?.id === selected.id}
            onToggle={(enabled) => toggle.mutate({ id: selected.id, enabled, name: keyLabel(selected) })}
            onRevoke={() => setDeleteTarget(selected)}
          />
        ) : null}
      </DetailSheet>
    </SettingsPage>
  );
}

function ApiKeyDetail({
  apiKey,
  index,
  total,
  onStep,
  toggling,
  onToggle,
  onRevoke,
}: {
  apiKey: ApiKeyRecord;
  index: number;
  total: number;
  onStep: (delta: -1 | 1) => void;
  toggling: boolean;
  onToggle: (enabled: boolean) => void;
  onRevoke: () => void;
}) {
  const [tab, setTab] = useState("overview");
  const json = useMemo(() => JSON.stringify(apiKey, null, 2), [apiKey]);
  const { copy } = useCopyToClipboard();
  const scoped = apiKey.actsAsCollection !== "";
  const label = keyLabel(apiKey);
  return (
    <>
      <DetailSheetHeader
        id={apiKey.id}
        pager={{
          index: index + 1,
          total,
          hasPrev: index > 0,
          hasNext: index < total - 1,
          onPrev: () => onStep(-1),
          onNext: () => onStep(1),
        }}
        menu={[
          { label: "Copy key prefix", onSelect: () => void copy(`cb_${apiKey.prefix}`), icon: <Copy /> },
          { label: "Copy JSON", onSelect: () => void copy(json), icon: <Copy /> },
          { label: "Revoke key", onSelect: onRevoke, destructive: true, icon: <Trash2 /> },
        ]}
        title={apiKey.name || "Untitled"}
        status={
          <StatusBadge tone={apiKey.enabled ? "success" : "neutral"}>{apiKey.enabled ? "Enabled" : "Disabled"}</StatusBadge>
        }
        meta={[
          <span key="prefix" className="font-mono">cb_{apiKey.prefix}…</span>,
          scoped ? "Acts as record" : "Superuser",
          <span key="used">
            Last used <TimeAgo value={apiKey.lastUsedAt} fallback="never" />
          </span>,
        ]}
        actions={
          <>
            <Button
              size="sm"
              variant={apiKey.enabled ? "outline" : "default"}
              disabled={toggling}
              aria-label={apiKey.enabled ? `Disable ${label}` : `Enable ${label}`}
              onClick={() => onToggle(!apiKey.enabled)}
            >
              {toggling ? <Spinner /> : null}
              {apiKey.enabled ? "Disable" : "Enable"}
            </Button>
            <Button size="sm" variant="outline" onClick={onRevoke}>
              <Trash2 />
              Revoke
            </Button>
          </>
        }
      />
      <DetailTabs
        value={tab}
        onValueChange={setTab}
        tabs={[
          { value: "overview", label: "Overview" },
          { value: "raw", label: "Raw JSON" },
        ]}
      >
        <DetailTabPanel value="overview">
          <DetailBody>
            {!apiKey.enabled ? (
              <DetailCallout tone="info" title="Key disabled">
                Requests that present this key are rejected until it is enabled again.
              </DetailCallout>
            ) : !scoped ? (
              <DetailCallout tone="warning" title="Unscoped key">
                This key acts as a superuser with full access to the dashboard API. Prefer a key that acts as a record where you can.
              </DetailCallout>
            ) : null}

            <DetailSection title="Access">
              <DetailList>
                <DetailRow label="Acts as">
                  {scoped ? (
                    <DetailChip className="font-mono">
                      {apiKey.actsAsCollection}/{apiKey.actsAsRecord}
                    </DetailChip>
                  ) : (
                    <DetailChip>Superuser</DetailChip>
                  )}
                </DetailRow>
                {scoped ? (
                  <>
                    <DetailRow label="Collection">
                      <span className="font-mono text-xs break-all">{apiKey.actsAsCollection}</span>
                    </DetailRow>
                    <DetailRow label="Record">
                      <span className="font-mono text-xs break-all">{apiKey.actsAsRecord}</span>
                    </DetailRow>
                  </>
                ) : null}
              </DetailList>
            </DetailSection>

            <DetailSection title="Details">
              <DetailList>
                <DetailRow label="Name">{apiKey.name || <span className="text-muted-foreground">Untitled</span>}</DetailRow>
                <DetailRow label="Key">
                  <span className="font-mono text-xs">cb_{apiKey.prefix}…</span>
                </DetailRow>
                <DetailRow label="Enabled">{apiKey.enabled ? "Yes" : "No"}</DetailRow>
                <DetailRow label="Created" hint={parseServerDate(apiKey.created).toLocaleString()}>
                  <TimeAgo value={apiKey.created} />
                </DetailRow>
                <DetailRow label="Last used" hint={apiKey.lastUsedAt ? parseServerDate(apiKey.lastUsedAt).toLocaleString() : undefined}>
                  <TimeAgo value={apiKey.lastUsedAt} fallback="Never" />
                </DetailRow>
                <DetailRow label="ID">
                  <span className="font-mono text-xs break-all">{apiKey.id}</span>
                </DetailRow>
              </DetailList>
            </DetailSection>
          </DetailBody>
        </DetailTabPanel>
        <DetailTabPanel value="raw">
          <DetailBody>
            <CodeBlock title="Raw record" json={json} copyLabel="Copy JSON" />
          </DetailBody>
        </DetailTabPanel>
      </DetailTabs>
    </>
  );
}

/** What a key is called in confirmations and aria-labels; an unnamed legacy
 * key falls back to its visible prefix. */
function keyLabel(key: ApiKeyRecord): string {
  return key.name || `cb_${key.prefix}`;
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
  const [touched, setTouched] = useState(false);
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
            ? { name: name.trim(), actsAsCollection: targetCollection.name, actsAsRecord: recordId }
            : { name: name.trim() },
      }),
    onSuccess: onMinted,
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error("Couldn't create the API key", { description: described.serverMessage || described.detail });
    },
  });

  const nameError = touched && !name.trim() ? "Give the key a name so you can recognise it later." : null;
  const scopeReady = mode === "superuser" || (collectionId !== undefined && recordId !== undefined);
  const canCreate = Boolean(name.trim()) && scopeReady;

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form
          noValidate
          className="grid gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            setTouched(true);
            if (canCreate && !create.isPending) create.mutate();
          }}
        >
          <DialogHeader>
            <DialogTitle>New API key</DialogTitle>
            <DialogDescription>
              A bearer credential for scripts, CI jobs, or MCP clients. You'll see the key once, right after creating it.
            </DialogDescription>
          </DialogHeader>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="api-key-name">Name</Label>
            <Input
              id="api-key-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onBlur={() => setTouched(true)}
              placeholder="Deploy script"
              autoComplete="off"
              autoFocus
              aria-invalid={nameError ? true : undefined}
              aria-describedby={nameError ? "api-key-name-error" : "api-key-name-help"}
              className="h-control-md"
            />
            {nameError ? (
              <p id="api-key-name-error" role="alert" className="text-xs text-destructive">
                {nameError}
              </p>
            ) : (
              <p id="api-key-name-help" className="text-xs text-muted-foreground">
                For your own reference — it plays no part in authentication.
              </p>
            )}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label id="api-key-scope-label">Who does this key act as?</Label>
            <ToggleGroup
              type="single"
              variant="outline"
              size="sm"
              spacing={0}
              aria-labelledby="api-key-scope-label"
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
            {mode === "superuser" ? (
              <div className="flex gap-2 rounded-md border border-warning/30 bg-warning/12 px-2.5 py-2 text-xs text-pretty">
                <ShieldAlert aria-hidden className="mt-px size-3.5 shrink-0 text-warning" />
                <p>
                  <span className="font-medium">Unrestricted root.</span> Requests bypass every collection rule and can
                  read, change, or delete anything — never hand this key to untrusted code.
                </p>
              </div>
            ) : (
              <p className="text-xs text-pretty text-muted-foreground">
                The key gets exactly the access that record would have signing in normally — your collection rules still
                apply. Pick the least-privileged account that can do the job.
              </p>
            )}
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
                <SelectTrigger id="api-key-acts-as-collection" className="h-control-md w-full">
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
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={create.isPending || (touched && !canCreate)}>
              {create.isPending ? <Spinner className="size-3.5" /> : null}
              {create.isPending ? "Creating…" : "Create key"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** Shows the raw key exactly once — the server never sends it again
 * (`_api_keys.key` is `hidden` and stores only a hash). Because losing it
 * means minting a new key, neither Esc nor an outside click closes it
 * silently: they raise an inline "close anyway?" prompt, and the only
 * direct exit is the explicit "I've copied it" button. */
function RevealKeyDialog({ minted, onDone }: { minted: MintedApiKey; onDone: () => void }) {
  const keyCopy = useCopyToClipboard();
  const snippetCopy = useCopyToClipboard();
  const [hasCopied, setHasCopied] = useState(false);
  const [confirmingClose, setConfirmingClose] = useState(false);

  const endpoint = new URL(cb.buildURL("/api/collections/<collection>/records"), window.location.origin)
    .toString()
    .replace("%3Ccollection%3E", "<collection>");
  const snippet = `curl ${endpoint} \\\n  -H "Authorization: Bearer ${minted.key}"`;

  async function copyKey() {
    if (await keyCopy.copy(minted.key)) setHasCopied(true);
  }

  function requestClose(event: Event) {
    event.preventDefault();
    setConfirmingClose(true);
  }

  return (
    <Dialog open onOpenChange={() => setConfirmingClose(true)}>
      <DialogContent
        className="sm:max-w-lg"
        showCloseButton={false}
        onEscapeKeyDown={requestClose}
        onInteractOutside={requestClose}
      >
        <DialogHeader>
          <DialogTitle>Copy your new key</DialogTitle>
          <DialogDescription>
            This is the only time “{minted.name || "this key"}” is shown in full. Cratebase keeps just a hash of it — if
            you lose it, revoke it and create another.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="api-key-secret">API key</Label>
          <div className="flex items-stretch gap-1.5">
            <Input
              id="api-key-secret"
              readOnly
              value={minted.key}
              autoFocus
              spellCheck={false}
              onFocus={(event) => event.currentTarget.select()}
              className="h-control-md min-w-0 flex-1 font-mono text-xs"
            />
            <Button type="button" variant="outline" className="h-control-md" onClick={() => void copyKey()}>
              {keyCopy.status === "copied" ? <Check className="text-success" /> : <Copy />}
              {keyCopy.status === "copied" ? "Copied" : "Copy"}
            </Button>
          </div>
          <p role="status" className="min-h-4 text-xs text-muted-foreground">
            {keyCopy.status === "error" ? (
              <span className="text-destructive">Couldn't reach the clipboard — select the key and copy it manually.</span>
            ) : null}
          </p>
        </div>

        <div className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs font-medium">Use it</span>
            <Button
              type="button"
              variant="ghost"
              size="xs"
              aria-label="Copy curl example"
              onClick={() => void snippetCopy.copy(snippet)}
            >
              {snippetCopy.status === "copied" ? <Check className="text-success" /> : <Copy />}
              {snippetCopy.status === "copied" ? "Copied" : "Copy example"}
            </Button>
          </div>
          <pre className="overflow-x-auto rounded-lg border border-border bg-surface-sunken px-3 py-2.5 font-mono text-xs leading-relaxed text-foreground">
            {snippet}
          </pre>
          <p className="text-xs text-muted-foreground">Send it as a bearer token in the Authorization header.</p>
        </div>

        {confirmingClose ? (
          <div
            role="alert"
            className="flex flex-col gap-2 rounded-md border border-warning/30 bg-warning/12 px-3 py-2.5 text-sm"
          >
            <p>
              {hasCopied
                ? "Close this dialog? The key can't be shown again."
                : "You haven't copied the key yet. Close anyway? It can't be shown again."}
            </p>
            <div className="flex flex-wrap justify-end gap-2">
              <Button type="button" size="sm" variant="outline" onClick={() => setConfirmingClose(false)}>
                Keep open
              </Button>
              <Button type="button" size="sm" variant="destructive" onClick={onDone}>
                {hasCopied ? "Close" : "Close without copying"}
              </Button>
            </div>
          </div>
        ) : null}

        <DialogFooter>
          <Button type="button" onClick={onDone}>
            I've copied it
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
