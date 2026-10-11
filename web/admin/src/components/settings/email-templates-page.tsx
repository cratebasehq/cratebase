import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { Copy, Eye, FileText, Pencil, Plus, RotateCcw, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { cb, describeFailure } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { StatusBadge } from "@/components/layout/stat-card";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DetailBody,
  DetailSection,
  DetailSheet,
  DetailSheetHeader,
  DetailList,
  DetailRow,
  DetailTabPanel,
  DetailTabs,
} from "@/components/layout/detail-sheet";
import { CodeBlock } from "@/components/settings/detail-json";
import { TypedConfirmDialog } from "@/components/settings/typed-confirm-dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { SettingsPage } from "@/components/settings/settings-form";
import { TimeAgo } from "@/components/settings/time-ago";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

/** An `_emailTemplates` record, as `crates/server/src/mail_templates.rs`
 * reads/writes it — see that module and `crate::routes::mails`'s doc for
 * `sendRule`'s three states. */
interface TemplateRecord {
  id: string;
  key: string;
  name: string;
  subject: string;
  html: string;
  text: string;
  locale: string;
  layout: boolean;
  description: string;
  sendRule: string | null;
  editor: "visual" | "html" | "";
  updated?: string;
  created?: string;
}

type KindFilter = "all" | "system" | "custom";

/** Auth flows look their template up as `auth.<kind>`
 * (`crates/server/src/mail_templates.rs`); a row with such a key overrides
 * the built-in default, and deleting it restores the default. */
function isSystemTemplate(template: TemplateRecord): boolean {
  return template.key.startsWith("auth.");
}

function SendRuleBadge({ sendRule }: { sendRule: string | null }) {
  if (sendRule === null) return <StatusBadge tone="neutral">Superuser only</StatusBadge>;
  if (sendRule === "") return <StatusBadge tone="danger">Public</StatusBadge>;
  return <StatusBadge tone="info">Rule</StatusBadge>;
}

/** Preview, details and source of one template. Editing stays on its own
 * route; the header's Edit button goes there. The pager walks the filtered list. */
function TemplateDetailSheet({
  template,
  index,
  total,
  onClose,
  onStep,
  onDuplicate,
  duplicating,
  onDelete,
}: {
  template: TemplateRecord | null;
  index: number;
  total: number;
  onClose: () => void;
  onStep: (delta: -1 | 1) => void;
  onDuplicate: (template: TemplateRecord) => void;
  duplicating: boolean;
  onDelete: (template: TemplateRecord) => void;
}) {
  const [tab, setTab] = useState("preview");
  const preview = useQuery({
    queryKey: ["email-templates", "preview", template?.id, template?.updated],
    queryFn: () => cb.mails.preview({ template: template!.key, locale: template!.locale || undefined }),
    enabled: template !== null,
    retry: false,
  });
  const system = template ? isSystemTemplate(template) : false;
  return (
    <DetailSheet open={template !== null} onOpenChange={(open) => !open && onClose()}>
      {template ? (
        <>
          <DetailSheetHeader
            id={template.key}
            pager={{
              index: index + 1,
              total,
              hasPrev: index > 0,
              hasNext: index < total - 1,
              onPrev: () => onStep(-1),
              onNext: () => onStep(1),
            }}
            menu={[
              {
                label: system ? "Reset to default" : "Delete",
                destructive: true,
                icon: system ? <RotateCcw /> : <Trash2 />,
                onSelect: () => onDelete(template),
              },
            ]}
            title={template.name || template.key}
            description={`Template ${template.key}`}
            status={<StatusBadge tone={system ? "info" : "neutral"}>{system ? "System" : "Custom"}</StatusBadge>}
            meta={[
              <span key="locale" className="font-mono">{template.locale || "default"}</span>,
              <SendRuleBadge key="rule" sendRule={template.sendRule} />,
              <TimeAgo key="updated" value={template.updated} />,
            ]}
            actions={
              <>
                <Button size="sm" asChild>
                  <Link to="/settings/email-templates/$id" params={{ id: template.id }}>
                    <Pencil />
                    Edit template
                  </Link>
                </Button>
                <Button size="sm" variant="outline" disabled={duplicating} onClick={() => onDuplicate(template)}>
                  <Copy />
                  Duplicate
                </Button>
              </>
            }
          />
          <DetailTabs
            value={tab}
            onValueChange={setTab}
            tabs={[
              { value: "preview", label: "Preview" },
              { value: "details", label: "Details" },
              { value: "source", label: "Source" },
            ]}
          >
            <DetailTabPanel value="preview">
              <DetailBody>
                <p className="text-xs text-muted-foreground">
                  Rendered by the server with no template data, so variables may show empty. Open the editor to preview
                  with sample data.
                </p>
                {preview.isPending ? (
                  <Skeleton className="h-96 w-full" />
                ) : preview.error ? (
                  <ErrorState compact title="Couldn't render the preview" error={preview.error} onRetry={() => void preview.refetch()} />
                ) : (
                  <div className="flex flex-col gap-2">
                    <p className="text-sm break-words">
                      <span className="text-muted-foreground">Subject: </span>
                      {preview.data?.subject}
                    </p>
                    <iframe
                      title={`Preview of ${template.key}`}
                      srcDoc={preview.data?.html ?? ""}
                      // Untrusted HTML: no scripts, no same-origin, no popups. White
                      // like a mail client, regardless of the admin theme.
                      sandbox=""
                      className="h-[28rem] w-full rounded-md border border-border bg-white"
                    />
                  </div>
                )}
              </DetailBody>
            </DetailTabPanel>
            <DetailTabPanel value="details">
              <DetailBody>
                <DetailSection title="Template">
                  <DetailList>
                    <DetailRow label="Key">
                      <span className="font-mono text-xs break-all">{template.key}</span>
                    </DetailRow>
                    <DetailRow label="Name">{template.name || "—"}</DetailRow>
                    <DetailRow label="Subject">
                      {template.subject ? <span className="break-words">{template.subject}</span> : "(none)"}
                    </DetailRow>
                    <DetailRow label="Description">{template.description || "—"}</DetailRow>
                    <DetailRow label="Locale">
                      <span className="font-mono text-xs">{template.locale || "default"}</span>
                    </DetailRow>
                    <DetailRow label="Layout" hint={template.layout ? "wrapped in the shared mail layout" : undefined}>
                      {template.layout ? "Yes" : "No"}
                    </DetailRow>
                    <DetailRow label="Editor">{template.editor || "—"}</DetailRow>
                  </DetailList>
                </DetailSection>
                <DetailSection title="Access">
                  <DetailList>
                    <DetailRow label="Who can send it">
                      <SendRuleBadge sendRule={template.sendRule} />
                    </DetailRow>
                    {template.sendRule ? (
                      <DetailRow label="Send rule">
                        <span className="font-mono text-xs break-all">{template.sendRule}</span>
                      </DetailRow>
                    ) : null}
                  </DetailList>
                </DetailSection>
                <DetailSection title="Timing">
                  <DetailList>
                    <DetailRow label="Edited" hint={template.updated ? new Date(template.updated).toLocaleString() : undefined}>
                      <TimeAgo value={template.updated} />
                    </DetailRow>
                    <DetailRow label="Created" hint={template.created ? new Date(template.created).toLocaleString() : undefined}>
                      <TimeAgo value={template.created} />
                    </DetailRow>
                  </DetailList>
                </DetailSection>
              </DetailBody>
            </DetailTabPanel>
            <DetailTabPanel value="source">
              <DetailBody>
                <CodeBlock title="HTML" plain={template.html} copyLabel="Copy HTML" />
                <CodeBlock title="Plain text" plain={template.text} copyLabel="Copy text" />
              </DetailBody>
            </DetailTabPanel>
          </DetailTabs>
        </>
      ) : null}
    </DetailSheet>
  );
}

/**
 * List of `_emailTemplates` rows — the emails this instance sends, and
 * (via `sendRule`) who besides a superuser/API key may trigger one
 * through `POST /api/mails/send`. Editing happens on its own route
 * (`EmailTemplateEditorPage`); this page is create/duplicate/delete plus
 * the list and a quick preview.
 */
export function EmailTemplatesPage() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [q, setQ] = useState("");
  const [kind, setKind] = useState<KindFilter>("all");
  const [deleteTarget, setDeleteTarget] = useState<TemplateRecord | null>(null);

  const [selectedId, setSelectedId] = useState<string | null>(null);

  const { data, isPending, error, refetch } = useQuery({
    queryKey: ["email-templates"],
    queryFn: () => cb.collection("_emailTemplates").fullList({ sort: "key,locale" }) as unknown as Promise<TemplateRecord[]>,
  });

  const duplicate = useMutation({
    mutationFn: async (template: TemplateRecord) => {
      const key = `${template.key}-copy`;
      return cb.collection("_emailTemplates").create({
        key,
        name: `${template.name} (copy)`,
        subject: template.subject,
        html: template.html,
        text: template.text,
        locale: template.locale,
        layout: template.layout,
        description: template.description,
        sendRule: null,
        editor: template.editor || "html",
      });
    },
    onSuccess: (created, template) => {
      toast.success(`Duplicated “${template.key}” as “${template.key}-copy”`);
      void queryClient.invalidateQueries({ queryKey: ["email-templates"] });
      const record = created as unknown as { id: string };
      void navigate({ to: "/settings/email-templates/$id", params: { id: record.id } });
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error("Couldn't duplicate the template", {
        description: Object.values(described.fields)[0] ?? described.serverMessage ?? described.detail,
      });
    },
  });

  const remove = useMutation({
    mutationFn: (template: TemplateRecord) => cb.collection("_emailTemplates").delete(template.id),
    onSuccess: (_result, template) => {
      toast.success(isSystemTemplate(template) ? `Reset “${template.key}” to the built-in default` : `Deleted “${template.key}”`);
      void queryClient.invalidateQueries({ queryKey: ["email-templates"] });
      closeDelete();
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error("Couldn't remove the template", { description: described.serverMessage || described.detail });
    },
  });

  function closeDelete() {
    setDeleteTarget(null);
  }

  const templates = useMemo(() => data ?? [], [data]);
  const visible = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return templates.filter((template) => {
      if (kind === "system" && !isSystemTemplate(template)) return false;
      if (kind === "custom" && isSystemTemplate(template)) return false;
      if (!needle) return true;
      return [template.key, template.name, template.subject].some((text) => text.toLowerCase().includes(needle));
    });
  }, [templates, q, kind]);
  const item = settingsItemFor("/settings/email-templates")!;
  const selectedIndex = selectedId ? visible.findIndex((t) => t.id === selectedId) : -1;
  const selected = selectedIndex >= 0 ? visible[selectedIndex] : null;
  function step(delta: -1 | 1) {
    const next = visible[selectedIndex + delta];
    if (next) setSelectedId(next.id);
  }
  const deleteIsReset = deleteTarget ? isSystemTemplate(deleteTarget) : false;


  const newTemplateButton = (
    <Button size="sm" asChild>
      <Link to="/settings/email-templates/$id" params={{ id: "new" }}>
        <Plus />
        New template
      </Link>
    </Button>
  );

  return (
    <SettingsPage title={item.label} description={item.description} width="wide" action={newTemplateButton}>
      {templates.length > 0 ? (
        <DataTableToolbar
          search={{ value: q, onChange: setQ, placeholder: "Search by key, name or subject…" }}
          filters={
            <Select value={kind} onValueChange={(next) => setKind(next as KindFilter)}>
              <SelectTrigger aria-label="Filter by kind" className="h-control-sm w-36">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All templates</SelectItem>
                <SelectItem value="system">System</SelectItem>
                <SelectItem value="custom">Custom</SelectItem>
              </SelectContent>
            </Select>
          }
          actions={
            <span className="font-tabular text-xs text-muted-foreground">
              {visible.length} of {templates.length}
            </span>
          }
        />
      ) : null}

      {error && !data ? (
        <ErrorState title="Couldn't load templates" error={error} onRetry={() => void refetch()} />
      ) : isPending ? (
        <ListSkeleton rows={5} />
      ) : templates.length === 0 ? (
        <EmptyState
          compact
          icon={<FileText />}
          title="No templates yet"
          description="Templates are the branded, editable emails this app sends. Auth emails (auth.verification, auth.passwordReset…) use a built-in default until you create a template with that key."
          action={newTemplateButton}
        />
      ) : visible.length === 0 ? (
        <EmptyState
          compact
          icon={<FileText />}
          title="No templates match"
          description="Try a different search or kind."
          action={
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                setQ("");
                setKind("all");
              }}
            >
              Clear filters
            </Button>
          }
        />
      ) : (
        <div className="overflow-x-auto rounded-lg border border-border bg-card">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Template</TableHead>
                <TableHead>Subject</TableHead>
                <TableHead>Kind</TableHead>
                <TableHead>Locale</TableHead>
                <TableHead>Who can send it</TableHead>
                <TableHead>Edited</TableHead>
                <TableHead className="w-40 text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {visible.map((template) => {
                const system = isSystemTemplate(template);
                return (
                  <TableRow
                    key={template.id}
                    tabIndex={0}
                    role="button"
                    aria-label={`${template.key}, open details`}
                    data-state={template.id === selectedId ? "selected" : undefined}
                    onClick={(event) => {
                      if ((event.target as HTMLElement).closest("a,button")) return;
                      setSelectedId(template.id);
                    }}
                    onKeyDown={(event) => {
                      if (event.target === event.currentTarget && (event.key === "Enter" || event.key === " ")) {
                        event.preventDefault();
                        setSelectedId(template.id);
                      }
                    }}
                    className="cursor-pointer"
                  >
                    <TableCell>
                      <div className="flex flex-col">
                        <Link
                          to="/settings/email-templates/$id"
                          params={{ id: template.id }}
                          className="w-fit rounded-sm font-mono text-sm font-medium hover:underline"
                        >
                          {template.key}
                        </Link>
                        {template.name ? <span className="text-xs text-muted-foreground">{template.name}</span> : null}
                      </div>
                    </TableCell>
                    <TableCell className="max-w-64 truncate text-sm" title={template.subject}>
                      {template.subject || <span className="text-muted-foreground italic">(none)</span>}
                    </TableCell>
                    <TableCell>
                      <StatusBadge tone={system ? "info" : "neutral"}>{system ? "System" : "Custom"}</StatusBadge>
                    </TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">{template.locale || "default"}</TableCell>
                    <TableCell>
                      <SendRuleBadge sendRule={template.sendRule} />
                    </TableCell>
                    <TableCell className="text-xs whitespace-nowrap text-muted-foreground">
                      <TimeAgo value={template.updated} />
                    </TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-0.5">
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label={`Preview ${template.key}`}
                          title="Preview"
                          onClick={() => setSelectedId(template.id)}
                        >
                          <Eye />
                        </Button>
                        <Button variant="ghost" size="icon-sm" aria-label={`Edit ${template.key}`} title="Edit" asChild>
                          <Link to="/settings/email-templates/$id" params={{ id: template.id }}>
                            <Pencil />
                          </Link>
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label={`Duplicate ${template.key}`}
                          title="Duplicate"
                          disabled={duplicate.isPending}
                          onClick={() => duplicate.mutate(template)}
                        >
                          <Copy />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label={system ? `Reset ${template.key} to default` : `Delete ${template.key}`}
                          title={system ? "Reset to default" : "Delete"}
                          onClick={() => setDeleteTarget(template)}
                        >
                          {system ? <RotateCcw /> : <Trash2 />}
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

      <TemplateDetailSheet
        template={selected}
        index={selectedIndex}
        total={visible.length}
        onClose={() => setSelectedId(null)}
        onStep={step}
        onDuplicate={(template) => duplicate.mutate(template)}
        duplicating={duplicate.isPending}
        onDelete={setDeleteTarget}
      />

      <TypedConfirmDialog
        open={deleteTarget !== null && !deleteIsReset}
        onOpenChange={(open) => !open && closeDelete()}
        title={`Delete “${deleteTarget?.key ?? ""}”?`}
        description="Any auth flow or trigger still pointing at this key falls back to the built-in default, if there is one, or fails to send. There is no undo."
        confirmValue={deleteTarget?.key ?? ""}
        confirmLabel="Delete template"
        pendingLabel="Deleting…"
        pending={remove.isPending}
        onConfirm={() => deleteTarget && remove.mutate(deleteTarget)}
      />

      <Dialog open={deleteTarget !== null && deleteIsReset} onOpenChange={(open) => !open && !remove.isPending && closeDelete()}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Reset “{deleteTarget?.key}” to the default?</DialogTitle>
            <DialogDescription>
              Your customised version is removed and auth emails go back to the built-in default template. There is no
              undo.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={closeDelete} disabled={remove.isPending}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={remove.isPending}
              onClick={() => deleteTarget && remove.mutate(deleteTarget)}
            >
              {remove.isPending ? <Spinner /> : null}
              Reset to default
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsPage>
  );
}
