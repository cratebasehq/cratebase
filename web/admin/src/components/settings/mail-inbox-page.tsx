import { useMemo } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ArrowRight, Inbox, Trash2 } from "lucide-react";
import { cb, describeFailure } from "@/lib/api";
import { useDevMailInboxAvailable } from "@/hooks/use-settings";
import { settingsItemFor } from "@/lib/settings-nav";
import { settingsMailInboxRoute } from "@/routes/settings-mail-inbox";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { SettingsPage } from "@/components/settings/settings-form";
import { Link } from "@tanstack/react-router";

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
 */
export function MailInboxPage() {
  const search = settingsMailInboxRoute.useSearch();
  const navigate = settingsMailInboxRoute.useNavigate();
  const queryClient = useQueryClient();
  const { data: available, isPending: availabilityPending } = useDevMailInboxAvailable();

  const { data, isLoading } = useQuery({
    queryKey: LIST_QUERY_KEY,
    queryFn: () => cb.send<{ items: MailSummary[] }>("/api/dev/mails"),
    // Polling, not realtime — this is a dev-only convenience, and a
    // few-second delay noticing a just-sent email is an acceptable
    // trade-off against a bespoke SSE topic.
    refetchInterval: 4000,
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
      void queryClient.invalidateQueries({ queryKey: LIST_QUERY_KEY });
      void navigate({ search: {} });
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.detail || failure.serverMessage || undefined });
    },
  });

  const items = useMemo(() => data?.items ?? [], [data]);
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
            This only holds mail while the server has no real transport configured. SMTP is set up, so outgoing
            mail actually goes out — there's nothing to capture here.{" "}
            <Link to="/settings/mail-storage" className="inline-flex items-center gap-1">
              Go to Mail & storage <ArrowRight className="size-3.5" />
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
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="h-control-sm gap-1.5"
          disabled={clear.isPending || items.length === 0}
          onClick={() => clear.mutate()}
        >
          <Trash2 className="size-3.5" />
          Clear inbox
        </Button>
      }
    >
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(280px,360px)_1fr]">
        <div className="overflow-hidden rounded-lg border border-border bg-card">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Subject</TableHead>
                <TableHead>Sent</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {items.map((mail) => (
                <TableRow
                  key={mail.id}
                  data-state={mail.id === search.id ? "selected" : undefined}
                  className="cursor-pointer"
                  onClick={() => void navigate({ search: { id: mail.id } })}
                >
                  <TableCell className="max-w-[220px] truncate" title={mail.subject}>
                    <div className="truncate font-medium">{mail.subject || "(no subject)"}</div>
                    <div className="truncate text-xs text-muted-foreground">
                      {mail.to.map(formatAddress).join(", ")}
                    </div>
                  </TableCell>
                  <TableCell className="whitespace-nowrap text-xs text-muted-foreground">
                    {new Date(mail.sentAt).toLocaleTimeString()}
                  </TableCell>
                </TableRow>
              ))}
              {isLoading
                ? Array.from({ length: 4 }).map((_, i) => (
                    <TableRow key={i}>
                      <TableCell colSpan={2}>
                        <Skeleton className="h-row w-full" />
                      </TableCell>
                    </TableRow>
                  ))
                : null}
            </TableBody>
          </Table>
          {!isLoading && items.length === 0 ? (
            <Empty className="py-8">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Inbox />
                </EmptyMedia>
                <EmptyTitle>Nothing sent yet</EmptyTitle>
                <EmptyDescription>
                  Trigger a verification, password-reset, or OTP email and it will show up here.
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : null}
        </div>

        <div className="min-h-[420px] rounded-lg border border-border bg-card p-4">
          {!search.id ? (
            <Empty className="h-full py-12">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Inbox />
                </EmptyMedia>
                <EmptyTitle>Select an email</EmptyTitle>
                <EmptyDescription>Pick one from the list to read it.</EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : detailLoading || !detail ? (
            <div className="flex flex-col gap-3">
              <Skeleton className="h-6 w-2/3" />
              <Skeleton className="h-4 w-1/3" />
              <Skeleton className="h-96 w-full" />
            </div>
          ) : (
            <div className="flex h-full flex-col gap-3">
              <div>
                <h2 className="text-sm font-medium">{detail.subject || "(no subject)"}</h2>
                <p className="text-xs text-muted-foreground">
                  From {formatAddress(detail.from)} to {detail.to.map(formatAddress).join(", ")} ·{" "}
                  {new Date(detail.sentAt).toLocaleString()}
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
                    className="h-full min-h-[420px] w-full rounded-md border border-border bg-white"
                  />
                </TabsContent>
                <TabsContent value="text" className="min-h-0 flex-1 overflow-auto">
                  {detail.text ? (
                    <pre className="h-full min-h-[420px] overflow-auto rounded-md border border-border bg-muted/30 p-3 text-sm whitespace-pre-wrap">
                      {detail.text}
                    </pre>
                  ) : (
                    <p className="p-3 text-sm text-muted-foreground">No plain-text alternative.</p>
                  )}
                </TabsContent>
              </Tabs>
            </div>
          )}
        </div>
      </div>
    </SettingsPage>
  );
}
