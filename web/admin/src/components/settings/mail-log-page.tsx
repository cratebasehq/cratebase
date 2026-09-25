import { useQuery } from "@tanstack/react-query";
import { MailWarning } from "lucide-react";
import { cb, describeFailure, parseServerDate } from "@/lib/api";
import { settingsMailLogRoute, type MailLogSearch } from "@/routes/settings-mail-log";
import { settingsItemFor } from "@/lib/settings-nav";
import { Badge } from "@/components/ui/badge";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { Pagination } from "@/components/ui/pagination";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { SettingsPage } from "@/components/settings/settings-form";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

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

const STATUS_VARIANT: Record<MailLogEntry["status"], "default" | "secondary" | "destructive"> = {
  sent: "default",
  queued: "secondary",
  failed: "destructive",
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
 */
export function MailLogPage() {
  const search = settingsMailLogRoute.useSearch();
  const navigate = settingsMailLogRoute.useNavigate();
  const page = search.page ?? 1;
  const status = search.status ?? "";
  const template = search.template ?? "";

  const filterParts: string[] = [];
  if (status) filterParts.push(`status = "${status}"`);
  if (template) filterParts.push(`template = "${template.replace(/"/g, '\\"')}"`);
  const filter = filterParts.join(" && ");

  const { data, isLoading, error } = useQuery({
    queryKey: ["mail-log", page, status, template],
    queryFn: () =>
      cb.collection("_mailLog").list({
        page,
        perPage: 30,
        filter: filter || undefined,
        sort: "-created",
      }) as unknown as Promise<MailLogResult>,
    placeholderData: (previous) => previous,
  });

  function updateSearch(patch: Partial<Omit<MailLogSearch, "page">>) {
    void navigate({
      search: (prev) => ({ ...prev, page: undefined, ...patch }),
      replace: true,
    });
  }

  const item = settingsItemFor("/settings/mail-log")!;
  const items = data?.items ?? [];

  return (
    <SettingsPage title={item.label} description={item.description} width="wide">
      <div className="flex flex-wrap items-end gap-2">
        <div className="flex flex-col gap-1">
          <label htmlFor="mail-log-status-filter" className="text-xs text-muted-foreground">
            Status
          </label>
          <Select
            value={status || "all"}
            onValueChange={(next) =>
              updateSearch({ status: next === "all" ? undefined : (next as MailLogSearch["status"]) })
            }
          >
            <SelectTrigger id="mail-log-status-filter" className="w-40">
              <SelectValue placeholder="All statuses" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All statuses</SelectItem>
              <SelectItem value="sent">Sent</SelectItem>
              <SelectItem value="failed">Failed</SelectItem>
              <SelectItem value="queued">Queued</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="flex flex-col gap-1">
          <label htmlFor="mail-log-template-filter" className="text-xs text-muted-foreground">
            Template
          </label>
          <Input
            id="mail-log-template-filter"
            value={template}
            onChange={(e) => updateSearch({ template: e.target.value || undefined })}
            placeholder="e.g. welcome"
            className="w-48 font-mono text-sm"
          />
        </div>
      </div>

      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <MailWarning />
            </EmptyMedia>
            <EmptyTitle>Couldn't load the mail log</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : isLoading && !data ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 4 }, (_, i) => (
            <Skeleton key={i} className="h-row w-full" />
          ))}
        </div>
      ) : items.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <MailWarning />
            </EmptyMedia>
            <EmptyTitle>No mail logged yet</EmptyTitle>
            <EmptyDescription>Every send through POST /api/mails/send shows up here.</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Status</TableHead>
              <TableHead>Template</TableHead>
              <TableHead>To</TableHead>
              <TableHead>Subject</TableHead>
              <TableHead>Error</TableHead>
              <TableHead>Sent</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {items.map((entry) => (
              <TableRow key={entry.id} className="align-top">
                <TableCell className="py-3">
                  <Badge variant={STATUS_VARIANT[entry.status]} className="capitalize">
                    {entry.status}
                  </Badge>
                </TableCell>
                <TableCell className="py-3 font-mono text-xs">{entry.template || <span className="italic text-muted-foreground">raw</span>}</TableCell>
                <TableCell className="max-w-xs truncate py-3 text-sm">{formatAddresses(entry.to)}</TableCell>
                <TableCell className="max-w-xs truncate py-3 text-sm">{entry.subject || "(no subject)"}</TableCell>
                <TableCell className="max-w-xs truncate py-3 text-xs text-destructive" title={entry.error}>
                  {entry.error}
                </TableCell>
                <TableCell className="whitespace-nowrap py-3 text-xs text-muted-foreground">
                  {parseServerDate(entry.created).toLocaleString()}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
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
    </SettingsPage>
  );
}
