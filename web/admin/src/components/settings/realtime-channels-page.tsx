import { useState } from "react";
import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { Pencil, Plus, Radio, Trash2, Users, Waypoints } from "lucide-react";
import { toast } from "sonner";
import { cb, describeFailure } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
import { RuleField } from "@/components/collections/rule-field";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { StatCard, StatusBadge, StatusDot } from "@/components/layout/stat-card";
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
import { Spinner } from "@/components/ui/spinner";
import { LIVE_INTERVAL_MS, LiveToggle } from "@/components/settings/live-toggle";
import { SettingsPage } from "@/components/settings/settings-form";
import { TypedConfirmDialog } from "@/components/settings/typed-confirm-dialog";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";


/** A `_channels` record, as `crates/server/src/realtime.rs`'s channel
 * authorization lookup reads it — `subscribeRule`/`publishRule` follow the
 * same `null` (superusers only) / `""` (anyone) / expression convention as
 * `_emailTemplates.sendRule`. */
interface ChannelRecord {
  id: string;
  name: string;
  subscribeRule: string | null;
  publishRule: string | null;
}

interface ChannelStats {
  subscribers: number;
  presence: number;
}

/** `GET /api/realtime/channels/{name}/stats` — this node's own live
 * counts, polled while this tab is open. Node-local: on a multi-node
 * deployment this shows whichever node answered the request, not a
 * cluster-wide total — see the endpoint's own doc comment. */
function statsQuery(name: string, polling: boolean) {
  return {
    queryKey: ["realtime-channel-stats", name],
    queryFn: () => cb.send<ChannelStats>(`/api/realtime/channels/${encodeURIComponent(name)}/stats`),
    refetchInterval: polling ? LIVE_INTERVAL_MS : (false as const),
  };
}

/** Who a rule lets through, as a chip. An open publish rule is a warning:
 * anonymous callers can then write to the channel. */
function RuleChip({ rule, openIsRisky }: { rule: string | null; openIsRisky?: boolean }) {
  if (rule === null) return <StatusBadge tone="neutral">Superusers</StatusBadge>;
  if (rule === "") return <StatusBadge tone={openIsRisky ? "warning" : "info"}>Anyone</StatusBadge>;
  return (
    <span className="block max-w-xs truncate font-mono text-xs text-muted-foreground" title={rule}>
      {rule}
    </span>
  );
}

/**
 * CRUD over `_channels` plus a live subscriber/presence inspector — the
 * only way to enable a `channel:<name>` topic on `POST /api/realtime` (no
 * matching row disables it by default, see `Collection::default_system_collections`'s
 * comment on `channels`). Same superuser-only trust tier as `_webhooks`/
 * `_cron_jobs` (see `WebhooksPage`/`CronJobsPage`) — an operator-configured
 * integration point, not something a non-superuser record should read or
 * edit.
 */
export function RealtimeChannelsPage() {
  const queryClient = useQueryClient();
  const [dialogChannel, setDialogChannel] = useState<ChannelRecord | "new" | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ChannelRecord | null>(null);
  const [search, setSearch] = useState("");
  const [live, setLive] = useState(true);

  const { data, isPending, error, refetch } = useQuery({
    queryKey: ["realtime-channels"],
    queryFn: () => cb.collection("_channels").fullList({ sort: "name" }) as unknown as Promise<ChannelRecord[]>,
  });

  const channels = data ?? [];
  const paused = dialogChannel !== null || deleteTarget !== null;
  const polling = live && !paused;

  const statResults = useQueries({ queries: channels.map((c) => statsQuery(c.name, polling)) });
  const statsByName = new Map(channels.map((c, i) => [c.name, statResults[i]]));
  const anyFetching = statResults.some((r) => r.isFetching);
  const anyData = statResults.some((r) => r.data);
  const allFailing = channels.length > 0 && statResults.every((r) => r.isError);
  const totalSubscribers = statResults.reduce((sum, r) => sum + (r.data?.subscribers ?? 0), 0);
  const totalPresence = statResults.reduce((sum, r) => sum + (r.data?.presence ?? 0), 0);

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection("_channels").delete(id),
    onSuccess: () => {
      toast.success(`Channel "${deleteTarget?.name ?? ""}" deleted`);
      void queryClient.invalidateQueries({ queryKey: ["realtime-channels"] });
      setDeleteTarget(null);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(`Couldn't delete channel: ${described.title}`, { description: described.detail });
    },
  });

  const needle = search.trim().toLowerCase();
  const visible = needle ? channels.filter((c) => c.name.toLowerCase().includes(needle)) : channels;

  const newButton = (
    <Button size="sm" onClick={() => setDialogChannel("new")}>
      <Plus />
      New channel
    </Button>
  );

  const item = settingsItemFor("/settings/realtime")!;

  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={channels.length > 0 ? newButton : undefined}
    >
      {error ? (
        <ErrorState title="Couldn't load channels" error={error} onRetry={() => void refetch()} compact />
      ) : isPending ? (
        <ListSkeleton rows={3} />
      ) : channels.length === 0 ? (
        <EmptyState
          compact
          icon={<Radio />}
          title="No channels configured"
          description={
            'With no matching row every channel is disabled. Add one — an exact name ("room") or a prefix pattern ("room:*") — to let clients subscribe, publish, or track presence on it.'
          }
          action={newButton}
        />
      ) : (
        <>
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
            <StatCard label="Channels" value={channels.length} icon={<Waypoints />} index={0} />
            <StatCard
              label="Subscribers"
              value={anyData ? totalSubscribers : "–"}
              hint="Connected to this node"
              icon={<Radio />}
              index={1}
            />
            <StatCard label="Present" value={anyData ? totalPresence : "–"} hint="Tracked presence" icon={<Users />} index={2} />
          </div>

          <DataTableToolbar
            search={{ value: search, onChange: setSearch, placeholder: "Search channels…" }}
            actions={
              <LiveToggle
                live={polling}
                onLiveChange={setLive}
                fetching={anyFetching}
                onRefresh={() => void queryClient.invalidateQueries({ queryKey: ["realtime-channel-stats"] })}
              />
            }
          />
          {allFailing ? (
            <p role="alert" className="text-xs text-destructive">
              Couldn't read live counts — the realtime endpoint isn't responding. Retrying every {LIVE_INTERVAL_MS / 1000}s.
            </p>
          ) : null}

          {visible.length === 0 ? (
            <EmptyState compact title="No matching channels" description={`Nothing matches "${search}".`} />
          ) : (
            <div className="overflow-x-auto rounded-lg border border-border bg-card">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Name</TableHead>
                    <TableHead>Subscribe</TableHead>
                    <TableHead>Publish</TableHead>
                    <TableHead>Connections</TableHead>
                    <TableHead className="w-24 text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {visible.map((channel) => {
                    const stats = statsByName.get(channel.name);
                    return (
                      <TableRow key={channel.id}>
                        <TableCell className="font-mono text-sm font-medium">{channel.name}</TableCell>
                        <TableCell>
                          <RuleChip rule={channel.subscribeRule} />
                        </TableCell>
                        <TableCell>
                          <RuleChip rule={channel.publishRule} openIsRisky />
                        </TableCell>
                        <TableCell>
                          <div className="flex items-center gap-2 font-tabular text-xs text-muted-foreground">
                            <StatusDot
                              tone={stats?.isError ? "danger" : stats?.data?.subscribers ? "success" : "neutral"}
                              pulse={Boolean(stats?.data?.subscribers) && polling}
                            />
                            {stats?.isError ? (
                              <span className="text-destructive">Unavailable</span>
                            ) : (
                              <span>
                                {stats?.data ? stats.data.subscribers : "–"} subscriber
                                {stats?.data?.subscribers === 1 ? "" : "s"} · {stats?.data ? stats.data.presence : "–"} present
                              </span>
                            )}
                          </div>
                        </TableCell>
                        <TableCell>
                          <div className="flex justify-end gap-1">
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              aria-label={`Edit ${channel.name}`}
                              title="Edit"
                              onClick={() => setDialogChannel(channel)}
                            >
                              <Pencil />
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              aria-label={`Delete ${channel.name}`}
                              title="Delete"
                              onClick={() => setDeleteTarget(channel)}
                            >
                              <Trash2 />
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
          <p className="text-xs text-muted-foreground">
            Live counts are for the node that answered; on a multi-node deployment they aren't a cluster-wide total.
          </p>
        </>
      )}

      {dialogChannel ? (
        <ChannelDialog
          channel={dialogChannel === "new" ? null : dialogChannel}
          onOpenChange={(open) => {
            if (!open) setDialogChannel(null);
          }}
        />
      ) : null}

      <TypedConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => !open && setDeleteTarget(null)}
        title="Delete channel"
        description="Every current subscriber immediately loses access and clients can no longer publish to it. There is no undo."
        confirmValue={deleteTarget?.name ?? ""}
        confirmLabel="Delete channel"
        pendingLabel="Deleting…"
        pending={remove.isPending}
        onConfirm={() => deleteTarget && remove.mutate(deleteTarget.id)}
      />
    </SettingsPage>
  );
}

/** Create or edit one `_channels` record. `channel === null` means create. */
function ChannelDialog({
  channel,
  onOpenChange,
}: {
  channel: ChannelRecord | null;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [name, setName] = useState(channel?.name ?? "");
  const [subscribeRule, setSubscribeRule] = useState<string | null>(channel?.subscribeRule ?? null);
  const [publishRule, setPublishRule] = useState<string | null>(channel?.publishRule ?? null);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [submitted, setSubmitted] = useState(false);

  const trimmed = name.trim();
  // `*` is only meaningful as the last character of a prefix pattern.
  const nameProblem = !trimmed
    ? "Enter a channel name or prefix pattern"
    : /\s/.test(trimmed)
      ? "Names can't contain spaces"
      : trimmed.slice(0, -1).includes("*")
        ? "Use * only at the end, as in room:*"
        : undefined;
  const nameError = fieldErrors["name"] ?? (submitted ? nameProblem : undefined);

  const save = useMutation({
    mutationFn: () => {
      const body: Record<string, unknown> = { name: trimmed, subscribeRule, publishRule };
      return channel ? cb.collection("_channels").update(channel.id, body) : cb.collection("_channels").create(body);
    },
    onSuccess: () => {
      toast.success(channel ? `Channel "${trimmed}" updated` : `Channel "${trimmed}" created`);
      void queryClient.invalidateQueries({ queryKey: ["realtime-channels"] });
      onOpenChange(false);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      setFieldErrors(described.fields);
      if (Object.keys(described.fields).length === 0) {
        toast.error(`Couldn't save channel: ${described.title}`, { description: described.detail });
      }
    },
  });

  const isPrefix = trimmed.endsWith("*");

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-lg">
        <form
          className="flex flex-col gap-4"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            setSubmitted(true);
            if (!nameProblem) save.mutate();
          }}
        >
          <DialogHeader>
            <DialogTitle>{channel ? "Edit channel" : "New channel"}</DialogTitle>
            <DialogDescription>
              Clients can only use a channel once a row allows it, and each action is gated by its own rule.
            </DialogDescription>
          </DialogHeader>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="channel-name">Name or prefix pattern</Label>
            <Input
              id="channel-name"
              className="font-mono"
              value={name}
              autoFocus
              autoComplete="off"
              spellCheck={false}
              onChange={(e) => {
                setName(e.target.value);
                setFieldErrors(({ name: _name, ...rest }) => rest);
              }}
              placeholder="room, or room:*"
              aria-invalid={nameError ? true : undefined}
              aria-describedby="channel-name-note"
            />
            {nameError ? (
              <p id="channel-name-note" role="alert" className="text-xs text-destructive">
                {nameError}
              </p>
            ) : (
              <p id="channel-name-note" className="text-xs text-muted-foreground">
                {isPrefix ? (
                  <>
                    Matches every <span className="font-mono">channel:{trimmed.slice(0, -1)}…</span> topic and exposes the
                    matched part to rules as <span className="font-mono">@request.data.suffix</span>.
                  </>
                ) : trimmed ? (
                  <>
                    Matches only the <span className="font-mono">channel:{trimmed}</span> topic. End with{" "}
                    <span className="font-mono">*</span> to match a whole family.
                  </>
                ) : (
                  <>
                    An exact name (<span className="font-mono">room</span>) matches only that channel; a prefix pattern (
                    <span className="font-mono">room:*</span>) matches any topic that starts with it.
                  </>
                )}
              </p>
            )}
          </div>

          <RuleField
            label="Who can subscribe / track presence"
            value={subscribeRule}
            onChange={setSubscribeRule}
            nullOption={{ label: "Superusers", description: "Only a superuser or API key may subscribe or track presence." }}
            publicOption={{
              label: "Anyone",
              description: "Any caller, including anonymous ones, may subscribe or track presence.",
            }}
          />
          {fieldErrors["subscribeRule"] ? (
            <p role="alert" className="-mt-2 text-xs text-destructive">
              {fieldErrors["subscribeRule"]}
            </p>
          ) : null}

          <RuleField
            label="Who can publish"
            value={publishRule}
            onChange={setPublishRule}
            nullOption={{ label: "Superusers", description: "Only a superuser or API key may publish." }}
            publicOption={{
              label: "Anyone",
              description: "Any caller, including anonymous ones, may publish. Use with care.",
            }}
          />
          {fieldErrors["publishRule"] ? (
            <p role="alert" className="-mt-2 text-xs text-destructive">
              {fieldErrors["publishRule"]}
            </p>
          ) : null}

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {save.isPending ? <Spinner /> : null}
              {channel ? "Save changes" : "Create channel"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
