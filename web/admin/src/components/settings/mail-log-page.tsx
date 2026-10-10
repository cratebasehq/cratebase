import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { MailWarning, RotateCw } from "lucide-react";
import { cb, describeFailure, parseServerDate } from "@/lib/api";
import { settingsEmailRoute, type EmailSearch } from "@/routes/settings-email";
import { settingsItemFor } from "@/lib/settings-nav";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { StatusBadge, type Tone } from "@/components/layout/stat-card";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { Button } from "@/components/ui/button";
import { Pagination } from "@/components/ui/pagination";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Spinner } from "@/components/ui/spinner";
import { LiveToggle, LIVE_INTERVAL_MS } from "@/components/settings/live-toggle";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { SettingsPage } from "@/components/settings/settings-form";
import { TimeAgo } from "@/components/settings/time-ago";

interface MailAddress {
  address: string;
  name: string;
}

/** A `_mailLog` record, as `crates/server/src/mails.rs` writes it. */
interface MailLogEntry {
  id: string;
  to: MailAddress[];
  subject: string;
  template: string;
  status: "queued" | "sent" | "failed";
  error: string;
  messageId: string;
  created: string;
}

interface MailLogResult {
  page: number;
  perPage: number;
  totalItems: number;
  totalPages: number;
  items: MailLogEntry[];
}

const STATUS_TONE: Record<MailLogEntry["status"], Tone> = {
  sent: "success",
  queued: "warning",
  failed: "danger",
};

const STATUS_LABEL: Record<MailLogEntry["status"], string> = {
  sent: "Sent",
  queued: "Queued",
  failed: "Failed",
};



function formatAddresses(to: MailAddress[]): string {
  return to.map((a) => a.address).join(", ") || "(none)";
}

/**
 * Superuser-only, read-only view over `_mailLog` — every send attempt
 * through `POST /api/mails/send` (superuser, API key, a `sendRule`-
 * approved caller, or `crate::email_triggers`), newest first. `status`
 * always reflects the most recent delivery attempt (see
 * `crates/server/src/mails.rs`'s module doc): a `queued` row that never
 * resolves means the durable queue worker hasn't picked it up yet.
 *
 * The list polls while the tab is visible (react-query pauses background
 * polling on its own) so a send shows up without a manual reload.
 */
export function MailLogPage() {
  const search = settingsEmailRoute.useSearch();
  const navigate = settingsEmailRoute.useNavigate();
  const queryClient = useQueryClient();
  const page = search.page ?? 1;
  const status = search.status ?? "";
  const template = search.template ?? "";

  const [live, setLive] = useState(true);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [templateDraft, setTemplateDraft] = useState(template);

  // Debounce the free-text template filter into the URL.
  useEffect(() => {
    if (templateDraft === template) return;
    const timer = window.setTimeout(() => {
      void navigate({
        search: (prev) => ({ ...prev, page: undefined, template: templateDraft || undefined }),
        replace: true,
      });
    }, 250);
    return () => window.clearTimeout(timer);
  }, [templateDraft, template, navigate]);

  const filterParts: string[] = [];
  if (status) filterParts.push(`status = "${status}"`);
  if (template) filterParts.push(`template = "${template.replace(/"/g, '\\"')}"`);
  const filter = filterParts.join(" && ");

  const { data, isPending, error, refetch, isFetching } = useQuery({
    queryKey: ["mail-log", page, status, template],
    queryFn: () =>
      cb.collection("_mailLog").list({
        page,
        perPage: 30,
        filter: filter || undefined,
        sort: "-created",
      }) as unknown as Promise<MailLogResult>,
    placeholderData: (previous) => previous,
    refetchInterval: live && selectedId === null ? LIVE_INTERVAL_MS : false,
  });

  const retry = useMutation({
    mutationFn: (entry: MailLogEntry) =>
      cb.send<unknown>("/api/mails/send", {
        method: "POST",
        body: { to: entry.to.map((a) => a.address), template: entry.template },
      }),
    onSuccess: (_result, entry) => {
      toast.success(`Resent “${entry.template}” to ${formatAddresses(entry.to)}`, {
        description: "A new entry was added to the log.",
      });
      setSelectedId(null);
      void queryClient.invalidateQueries({ queryKey: ["mail-log"] });
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error("Couldn't resend the email", { description: described.serverMessage || described.detail });
    },
  });

  function updateSearch(patch: Partial<Omit<EmailSearch, "page" | "tab">>) {
    void navigate({
      search: (prev) => ({ ...prev, page: undefined, ...patch }),
      replace: true,
    });
  }

  const item = settingsItemFor("/settings/mail-log")!;
  const items = data?.items ?? [];
  const selected = items.find((entry) => entry.id === selectedId) ?? null;
  const filtered = Boolean(status || template);

  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={<LiveToggle live={live} onLiveChange={setLive} fetching={isFetching} onRefresh={() => void refetch()} />}
    >
      <DataTableToolbar
        search={{
          value: templateDraft,
          onChange: setTemplateDraft,
          placeholder: "Filter by template key…",
          label: "Filter by template key",
        }}
        filters={
          <Select
            value={status || "all"}
            onValueChange={(next) =>
              updateSearch({ status: next === "all" ? undefined : (next as EmailSearch["status"]) })
            }
          >
            <SelectTrigger aria-label="Filter by status" className="h-control-sm w-36">
              <SelectValue placeholder="All statuses" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All statuses</SelectItem>
              <SelectItem value="sent">Sent</SelectItem>
              <SelectItem value="failed">Failed</SelectItem>
              <SelectItem value="queued">Queued</SelectItem>
            </SelectContent>
          </Select>
        }
        actions={
          data ? (
            <span className="font-tabular text-xs text-muted-foreground">
              {data.totalItems} {data.totalItems === 1 ? "entry" : "entries"}
            </span>
          ) : null
        }
      />

      {error && !data ? (
        <ErrorState title="Couldn't load the mail log" error={error} onRetry={() => void refetch()} />
      ) : isPending ? (
        <ListSkeleton rows={6} />
      ) : items.length === 0 ? (
        <EmptyState
          compact
          icon={<MailWarning />}
          title={filtered ? "No mail matches these filters" : "No mail logged yet"}
          description={
            filtered
              ? "Try a different status or template key."
              : "Every send through POST /api/mails/send — and every auth email — shows up here."
          }
          action={
            filtered ? (
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  setTemplateDraft("");
                  updateSearch({ status: undefined, template: undefined });
                }}
              >
                Clear filters
              </Button>
            ) : undefined
          }
        />
      ) : (
        <div className="overflow-x-auto rounded-lg border border-border bg-card">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-24">Status</TableHead>
                <TableHead>Template</TableHead>
                <TableHead>To</TableHead>
                <TableHead>Subject</TableHead>
                <TableHead>Error</TableHead>
                <TableHead className="text-right">Sent</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {items.map((entry) => (
                <TableRow
                  key={entry.id}
                  data-state={entry.id === selectedId ? "selected" : undefined}
                  className="cursor-pointer"
                  onClick={() => setSelectedId(entry.id)}
                >
                  <TableCell>
                    <StatusBadge tone={STATUS_TONE[entry.status]}>{STATUS_LABEL[entry.status]}</StatusBadge>
                  </TableCell>
                  <TableCell className="font-mono text-xs">
                    {entry.template || <span className="font-sans text-muted-foreground italic">raw</span>}
                  </TableCell>
                  <TableCell className="max-w-56 truncate text-sm" title={formatAddresses(entry.to)}>
                    {formatAddresses(entry.to)}
                  </TableCell>
                  <TableCell className="max-w-64 truncate text-sm">
                    <button
                      type="button"
                      className="max-w-full truncate rounded-sm text-left hover:underline"
                      onClick={(event) => {
                        event.stopPropagation();
                        setSelectedId(entry.id);
                      }}
                    >
                      {entry.subject || "(no subject)"}
                    </button>
                  </TableCell>
                  <TableCell className="max-w-56 truncate text-xs text-destructive" title={entry.error}>
                    {entry.error}
                  </TableCell>
                  <TableCell className="text-right text-xs whitespace-nowrap text-muted-foreground">
                    <TimeAgo value={entry.created} />
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      {data && data.totalPages > 1 ? (
        <div className="flex justify-center py-2">
          <Pagination
            count={data.totalPages}
            page={page}
            onPageChange={(next) =>
              void navigate({ search: (prev) => ({ ...prev, page: next > 1 ? next : undefined }), replace: true })
            }
            label="Mail log pages"
          />
        </div>
      ) : null}

      <Sheet open={selected !== null} onOpenChange={(open) => !open && setSelectedId(null)}>
        <SheetContent className="w-full overflow-y-auto sm:max-w-md">
          {selected ? (
            <>
              <SheetHeader>
                <SheetTitle className="pr-8">{selected.subject || "(no subject)"}</SheetTitle>
                <SheetDescription>Details of this send attempt.</SheetDescription>
              </SheetHeader>
              <div className="flex flex-col gap-4 px-4 pb-4">
                <dl className="grid grid-cols-[6rem_1fr] gap-x-3 gap-y-2.5 text-sm">
                  <dt className="text-muted-foreground">Status</dt>
                  <dd>
                    <StatusBadge tone={STATUS_TONE[selected.status]}>{STATUS_LABEL[selected.status]}</StatusBadge>
                  </dd>
                  <dt className="text-muted-foreground">To</dt>
                  <dd className="font-mono text-xs break-all">
                    {selected.to.length === 0
                      ? "(none)"
                      : selected.to.map((a) => (a.name ? `${a.name} <${a.address}>` : a.address)).join(", ")}
                  </dd>
                  <dt className="text-muted-foreground">Template</dt>
                  <dd className="font-mono text-xs">
                    {selected.template || <span className="font-sans text-muted-foreground italic">raw (no template)</span>}
                  </dd>
                  <dt className="text-muted-foreground">Sent</dt>
                  <dd className="font-mono text-xs">{parseServerDate(selected.created).toLocaleString()}</dd>
                  <dt className="text-muted-foreground">Message ID</dt>
                  <dd className="font-mono text-xs break-all">
                    {selected.messageId || <span className="font-sans text-muted-foreground">Not assigned yet</span>}
                  </dd>
                  <dt className="text-muted-foreground">Log ID</dt>
                  <dd className="font-mono text-xs break-all">{selected.id}</dd>
                </dl>

                {selected.error ? (
                  <div
                    role="alert"
                    className="rounded-md border border-destructive/30 bg-destructive/10 p-3 font-mono text-xs break-words whitespace-pre-wrap text-destructive"
                  >
                    {selected.error}
                  </div>
                ) : null}

                {selected.status === "queued" ? (
                  <p className="text-xs text-muted-foreground">
                    Waiting for the queue worker. A row that stays queued means the worker hasn't picked it up yet.
                  </p>
                ) : null}

                {selected.status === "failed" ? (
                  selected.template ? (
                    <div className="flex flex-col gap-1.5">
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        className="self-start"
                        disabled={retry.isPending}
                        onClick={() => retry.mutate(selected)}
                      >
                        {retry.isPending ? <Spinner /> : <RotateCw />}
                        Retry send
                      </Button>
                      <p className="text-xs text-muted-foreground">
                        Sends “{selected.template}” again to the same recipients. The original template data isn't
                        stored, so placeholders that need it render empty.
                      </p>
                    </div>
                  ) : (
                    <p className="text-xs text-muted-foreground">
                      Raw sends can't be retried from here — the body isn't stored in the log. Send it again from the
                      original caller.
                    </p>
                  )
                ) : null}
              </div>
            </>
          ) : null}
        </SheetContent>
      </Sheet>
    </SettingsPage>
  );
}
