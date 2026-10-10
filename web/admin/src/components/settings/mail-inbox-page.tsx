import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ArrowRight, Inbox, Trash2 } from "lucide-react";
import { Link } from "@tanstack/react-router";
import { cb, describeFailure } from "@/lib/api";
import { useDevMailInboxAvailable } from "@/hooks/use-settings";
import { settingsItemFor } from "@/lib/settings-nav";
import { settingsEmailRoute } from "@/routes/settings-email";
import { LiveToggle, LIVE_INTERVAL_MS } from "@/components/settings/live-toggle";
import { EmptyState, ErrorState } from "@/components/layout/states";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Kbd } from "@/components/ui/kbd";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SettingsPage } from "@/components/settings/settings-form";
import { TimeAgo } from "@/components/settings/time-ago";
import { cn } from "@/lib/utils";

interface MailAddress {
  address: string;
  name: string;
}

interface MailSummary {
  id: string;
  to: MailAddress[];
  from: MailAddress;
  subject: string;
  sentAt: string;
}

interface MailDetail extends MailSummary {
  html: string;
  text: string | null;
}

function formatAddress(a: MailAddress): string {
  return a.name ? `${a.name} <${a.address}>` : a.address;
}

/** Injects `<base target="_blank">` so a link inside the captured email
 * opens in a new tab instead of navigating the sandboxed iframe itself —
 * needed because most transactional-email HTML doesn't set its own
 * `target` attribute. */
function withBaseTarget(html: string): string {
  if (/<head[\s>]/i.test(html)) {
    return html.replace(/<head([^>]*)>/i, `<head$1><base target="_blank">`);
  }
  if (/<html[\s>]/i.test(html)) {
    return html.replace(/<html([^>]*)>/i, `<html$1><head><base target="_blank"></head>`);
  }
  return `<!doctype html><html><head><base target="_blank"></head><body>${html}</body></html>`;
}

const LIST_QUERY_KEY = ["dev-mail-inbox", "list"] as const;


/**
 * Superuser-only view over the dev mail inbox (`/api/dev/mails`) — every
 * email the zero-config `Log` backend has "sent", newest first, with a
 * detail pane rendering the HTML body in a sandboxed iframe (no scripts,
 * links open in a new tab) alongside the plain-text alternative.
 *
 * Only reachable while the server is actually running the `Log` backend
 * — `useDevMailInboxAvailable` mirrors the same check the server itself
 * makes before answering these routes, so navigating here directly after
 * SMTP was turned on shows an explanation instead of a raw 404.
 *
 * Keyboard: with the list focused, ↑/↓ or j/k move between emails.
 */
export function MailInboxPage() {
  const search = settingsEmailRoute.useSearch();
  const navigate = settingsEmailRoute.useNavigate();
  const queryClient = useQueryClient();
  const { data: available, isPending: availabilityPending } = useDevMailInboxAvailable();
  const [clearOpen, setClearOpen] = useState(false);
  const [live, setLive] = useState(true);
  const listRef = useRef<HTMLDivElement>(null);

  const {
    data,
    isPending: listPending,
    error: listError,
    refetch,
    isFetching,
  } = useQuery({
    queryKey: LIST_QUERY_KEY,
    queryFn: () => cb.send<{ items: MailSummary[] }>("/api/dev/mails"),
    // Polling, not realtime — this is a dev-only convenience, and a
    // few-second delay noticing a just-sent email is an acceptable
    // trade-off against a bespoke SSE topic. react-query pauses it while
    // the tab is in the background.
    refetchInterval: live && !clearOpen ? LIVE_INTERVAL_MS : false,
    enabled: available === true,
  });

  const { data: detail, isLoading: detailLoading } = useQuery({
    queryKey: ["dev-mail-inbox", "detail", search.id],
    queryFn: () => cb.send<MailDetail>(`/api/dev/mails/${search.id}`),
    enabled: available === true && Boolean(search.id),
  });

  const clear = useMutation({
    mutationFn: () => cb.send<void>("/api/dev/mails", { method: "DELETE" }),
    onSuccess: () => {
      toast.success("Inbox cleared");
      setClearOpen(false);
      void queryClient.invalidateQueries({ queryKey: LIST_QUERY_KEY });
      void navigate({ search: (prev) => ({ tab: prev.tab }) });
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error("Couldn't clear the inbox", {
        description: failure.serverMessage || failure.detail || failure.title,
      });
    },
  });

  const items = useMemo(() => data?.items ?? [], [data]);
  const selectedIndex = items.findIndex((mail) => mail.id === search.id);

  // Keep the keyboard-selected row in view.
  useEffect(() => {
    if (!search.id) return;
    listRef.current?.querySelector(`[data-mail-id="${CSS.escape(search.id)}"]`)?.scrollIntoView({ block: "nearest" });
  }, [search.id]);

  function select(id: string) {
    void navigate({ search: (prev) => ({ ...prev, id }), replace: true });
  }

  function onListKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.metaKey || event.ctrlKey || event.altKey || items.length === 0) return;
    let next = -1;
    if (event.key === "ArrowDown" || event.key === "j") next = Math.min(items.length - 1, selectedIndex + 1);
    else if (event.key === "ArrowUp" || event.key === "k") next = Math.max(0, selectedIndex - 1);
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = items.length - 1;
    if (next < 0) return;
    event.preventDefault();
    select(items[next]!.id);
  }

  const item = settingsItemFor("/settings/mail-inbox")!;

  if (availabilityPending) {
    return (
      <SettingsPage title={item.label} description={item.description} width="wide">
        <Skeleton className="h-64 w-full" />
      </SettingsPage>
    );
  }

  if (!available) {
    return (
      <SettingsPage title={item.label} description={item.description} width="wide">
        <Alert>
          <Inbox />
          <AlertTitle>No dev inbox right now</AlertTitle>
          <AlertDescription>
            The inbox only holds mail while the server uses the built-in Log backend (no SMTP configured). SMTP is
            set up, so outgoing mail is really delivered and there's nothing to capture here.{" "}
            <Link to="/settings/email" search={{ tab: "delivery" }} className="inline-flex items-center gap-1">
              Go to Delivery <ArrowRight className="size-3.5" />
            </Link>
          </AlertDescription>
        </Alert>
      </SettingsPage>
    );
  }

  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={
        <>
          <LiveToggle live={live} onLiveChange={setLive} fetching={isFetching} onRefresh={() => void refetch()} />
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={clear.isPending || items.length === 0}
            onClick={() => setClearOpen(true)}
          >
            <Trash2 />
            Clear inbox
          </Button>
        </>
      }
    >
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(280px,360px)_1fr]">
        <div className="flex min-w-0 flex-col gap-1.5">
          <div
            ref={listRef}
            role="listbox"
            tabIndex={0}
            aria-label="Captured emails"
            aria-activedescendant={search.id && selectedIndex >= 0 ? `mail-${search.id}` : undefined}
            onKeyDown={onListKeyDown}
            className="max-h-[32rem] overflow-y-auto rounded-lg border border-border bg-card focus-visible:ring-3 focus-visible:ring-ring/40"
          >
            {listError && !data ? (
              <ErrorState compact title="Couldn't load the inbox" error={listError} onRetry={() => void refetch()} />
            ) : listPending ? (
              <div className="flex flex-col gap-px p-2">
                {Array.from({ length: 5 }, (_, i) => (
                  <Skeleton key={i} className="h-12 w-full" />
                ))}
              </div>
            ) : items.length === 0 ? (
              <EmptyState
                compact
                icon={<Inbox />}
                title="Nothing captured yet"
                description="While no SMTP server is configured, the Log backend keeps outgoing mail here instead of sending it. Trigger a verification, password-reset or OTP email and it appears within a few seconds."
                action={
                  <Button variant="outline" size="sm" asChild>
                    <Link to="/settings/email" search={{ tab: "delivery" }}>
                      Configure delivery
                    </Link>
                  </Button>
                }
              />
            ) : (
              items.map((mail) => {
                const active = mail.id === search.id;
                return (
                  <div
                    key={mail.id}
                    id={`mail-${mail.id}`}
                    data-mail-id={mail.id}
                    role="option"
                    aria-selected={active}
                    onClick={() => select(mail.id)}
                    className={cn(
                      "flex cursor-pointer flex-col gap-0.5 border-b border-border px-3 py-2 transition-colors duration-fast ease-out-strong last:border-b-0 hover:bg-muted/60",
                      active && "bg-brand-soft",
                    )}
                  >
                    <div className="flex items-baseline justify-between gap-2">
                      <span className="truncate text-sm font-medium">{mail.subject || "(no subject)"}</span>
                      <TimeAgo value={mail.sentAt} className="shrink-0 text-xs text-muted-foreground" />
                    </div>
                    <span className="truncate font-mono text-xs text-muted-foreground">
                      {mail.to.map(formatAddress).join(", ") || "(no recipients)"}
                    </span>
                  </div>
                );
              })
            )}
          </div>
          {items.length > 0 ? (
            <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <Kbd>↑</Kbd>
              <Kbd>↓</Kbd> or <Kbd>j</Kbd>
              <Kbd>k</Kbd> to move through the list
            </p>
          ) : null}
        </div>

        <div className="min-h-[420px] min-w-0 rounded-lg border border-border bg-card p-4">
          {!search.id ? (
            <EmptyState
              compact
              icon={<Inbox />}
              title="Select an email"
              description="Pick one from the list to read it."
              className="h-full"
            />
          ) : detailLoading || !detail ? (
            <div className="flex flex-col gap-3">
              <Skeleton className="h-6 w-2/3" />
              <Skeleton className="h-4 w-1/3" />
              <Skeleton className="h-96 w-full" />
            </div>
          ) : (
            <div className="flex h-full flex-col gap-3">
              <div className="flex flex-col gap-0.5">
                <h3 className="text-base font-medium">{detail.subject || "(no subject)"}</h3>
                <p className="text-xs text-muted-foreground">
                  From <span className="font-mono">{formatAddress(detail.from)}</span> to{" "}
                  <span className="font-mono">{detail.to.map(formatAddress).join(", ")}</span> ·{" "}
                  <TimeAgo value={detail.sentAt} className="font-sans" />
                </p>
              </div>
              <Tabs defaultValue="html" className="flex min-h-0 flex-1 flex-col">
                <TabsList>
                  <TabsTrigger value="html">HTML</TabsTrigger>
                  <TabsTrigger value="text">Text</TabsTrigger>
                </TabsList>
                <TabsContent value="html" className="min-h-0 flex-1">
                  <iframe
                    title="Email preview"
                    srcDoc={withBaseTarget(detail.html)}
                    // No scripts, no same-origin — this renders untrusted
                    // HTML. `allow-popups`/`allow-popups-to-escape-sandbox`
                    // are only what's needed for a `target="_blank"` link
                    // (via the injected `<base>` above) to actually open a
                    // real browser tab instead of being blocked outright.
                    sandbox="allow-popups allow-popups-to-escape-sandbox"
                    // An email is its own document; it is drawn on white
                    // regardless of the admin theme, as a mail client would.
                    className="h-full min-h-[420px] w-full rounded-md border border-border bg-white"
                  />
                </TabsContent>
                <TabsContent value="text" className="min-h-0 flex-1 overflow-auto">
                  {detail.text ? (
                    <pre className="h-full min-h-[420px] overflow-auto rounded-md border border-border bg-surface-sunken p-3 font-mono text-xs whitespace-pre-wrap">
                      {detail.text}
                    </pre>
                  ) : (
                    <p className="p-3 text-sm text-muted-foreground">This email has no plain-text alternative.</p>
                  )}
                </TabsContent>
              </Tabs>
            </div>
          )}
        </div>
      </div>

      <Dialog open={clearOpen} onOpenChange={(open) => !clear.isPending && setClearOpen(open)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Clear the dev inbox?</DialogTitle>
            <DialogDescription>
              This removes all {items.length} captured {items.length === 1 ? "email" : "emails"} from memory. They
              were never delivered, so there's no undo.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setClearOpen(false)} disabled={clear.isPending}>
              Cancel
            </Button>
            <Button variant="destructive" disabled={clear.isPending} onClick={() => clear.mutate()}>
              {clear.isPending ? <Spinner /> : null}
              Clear inbox
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsPage>
  );
}
