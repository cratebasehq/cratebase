import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Pencil, Plus, Radio, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { cb, describeFailure } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
import { RuleField } from "@/components/collections/rule-field";
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
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { SettingsPage } from "@/components/settings/settings-form";
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
function useChannelStats(name: string) {
  return useQuery({
    queryKey: ["realtime-channel-stats", name],
    queryFn: () => cb.send<ChannelStats>(`/api/realtime/channels/${encodeURIComponent(name)}/stats`),
    refetchInterval: 5000,
  });
}

function ChannelStatsBadges({ name }: { name: string }) {
  const { data } = useChannelStats(name);
  return (
    <div className="flex gap-1.5">
      <Badge variant="outline" className="font-normal text-muted-foreground">
        {data ? data.subscribers : "–"} subscriber{data?.subscribers === 1 ? "" : "s"}
      </Badge>
      <Badge variant="outline" className="font-normal text-muted-foreground">
        {data ? data.presence : "–"} present
      </Badge>
    </div>
  );
}

function ruleSummary(rule: string | null): string {
  if (rule === null) return "Superusers only";
  if (rule === "") return "Anyone";
  return rule;
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

  const { data, isLoading, error } = useQuery({
    queryKey: ["realtime-channels"],
    queryFn: () => cb.collection("_channels").fullList({ sort: "name" }) as unknown as Promise<ChannelRecord[]>,
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection("_channels").delete(id),
    onSuccess: () => {
      toast.success("Channel deleted");
      void queryClient.invalidateQueries({ queryKey: ["realtime-channels"] });
      setDeleteTarget(null);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const channels = data ?? [];
  const item = settingsItemFor("/settings/realtime")!;

  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={
        <Button size="sm" className="gap-1.5" onClick={() => setDialogChannel("new")}>
          <Plus className="size-3.5" />
          New channel
        </Button>
      }
    >
      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Radio />
            </EmptyMedia>
            <EmptyTitle>Couldn't load channels</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : isLoading ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 2 }, (_, i) => (
            <Skeleton key={i} className="h-row w-full" />
          ))}
        </div>
      ) : channels.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Radio />
            </EmptyMedia>
            <EmptyTitle>No channels configured</EmptyTitle>
            <EmptyDescription>
              With no matching row, every channel is disabled by default. Add one — an exact name ("room") or a prefix
              pattern ("room:*") — to let clients subscribe, publish, or track presence on it.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Subscribe</TableHead>
              <TableHead>Publish</TableHead>
              <TableHead>Live</TableHead>
              <TableHead className="w-24 text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {channels.map((channel) => (
              <TableRow key={channel.id} className="align-top">
                <TableCell className="py-3 font-mono text-sm font-medium">{channel.name}</TableCell>
                <TableCell className="max-w-xs truncate py-3 text-xs text-muted-foreground">
                  {ruleSummary(channel.subscribeRule)}
                </TableCell>
                <TableCell className="max-w-xs truncate py-3 text-xs text-muted-foreground">
                  {ruleSummary(channel.publishRule)}
                </TableCell>
                <TableCell className="py-3">
                  <ChannelStatsBadges name={channel.name} />
                </TableCell>
                <TableCell className="py-3 text-right">
                  <div className="flex justify-end gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Edit ${channel.name}`}
                      onClick={() => setDialogChannel(channel)}
                    >
                      <Pencil className="size-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Delete ${channel.name}`}
                      onClick={() => setDeleteTarget(channel)}
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      {dialogChannel ? (
        <ChannelDialog
          channel={dialogChannel === "new" ? null : dialogChannel}
          onOpenChange={(open) => {
            if (!open) setDialogChannel(null);
          }}
        />
      ) : null}

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete "{deleteTarget?.name}"?</DialogTitle>
            <DialogDescription>Every current subscriber immediately loses access. There is no undo.</DialogDescription>
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
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
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

  const save = useMutation({
    mutationFn: () => {
      const body: Record<string, unknown> = { name, subscribeRule, publishRule };
      return channel ? cb.collection("_channels").update(channel.id, body) : cb.collection("_channels").create(body);
    },
    onSuccess: () => {
      toast.success(channel ? "Channel updated" : "Channel created");
      void queryClient.invalidateQueries({ queryKey: ["realtime-channels"] });
      onOpenChange(false);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      setFieldErrors(described.fields);
      if (Object.keys(described.fields).length === 0) {
        toast.error(described.title, { description: described.detail });
      }
    },
  });

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{channel ? "Edit channel" : "New channel"}</DialogTitle>
          <DialogDescription>
            An exact name ("room") matches only that channel; a prefix pattern ("room:*") matches any
            "channel:room:..." topic and exposes the matched part to rules as @request.data.suffix.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="channel-name">Name or prefix pattern</Label>
            <Input
              id="channel-name"
              className="font-mono text-sm"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="room, or room:*"
            />
            {fieldErrors["name"] ? <p className="text-xs text-destructive">{fieldErrors["name"]}</p> : null}
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
            <p className="-mt-2 text-xs text-destructive">{fieldErrors["subscribeRule"]}</p>
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
            <p className="-mt-2 text-xs text-destructive">{fieldErrors["publishRule"]}</p>
          ) : null}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={save.isPending || !name} onClick={() => save.mutate()}>
            {save.isPending ? <Spinner className="size-3.5" /> : null}
            {channel ? "Save" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
