import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { Copy, FileText, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { cb, describeFailure } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
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
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { SettingsPage } from "@/components/settings/settings-form";
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
}

function SendRuleBadge({ sendRule }: { sendRule: string | null }) {
  if (sendRule === null) {
    return (
      <Badge variant="outline" className="font-normal text-muted-foreground">
        Superuser only
      </Badge>
    );
  }
  if (sendRule === "") {
    return (
      <Badge variant="destructive" className="font-normal">
        Public
      </Badge>
    );
  }
  return (
    <Badge variant="secondary" className="font-mono font-normal">
      Rule
    </Badge>
  );
}

/**
 * List of `_emailTemplates` rows — the emails this instance sends, and
 * (via `sendRule`) who besides a superuser/API key may trigger one
 * through `POST /api/mails/send`. Editing happens on its own route
 * (`EmailTemplateEditorPage`); this page is create/duplicate/delete plus
 * the list.
 */
export function EmailTemplatesPage() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [deleteTarget, setDeleteTarget] = useState<TemplateRecord | null>(null);

  const { data, isLoading, error } = useQuery({
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
    onSuccess: (created) => {
      toast.success("Template duplicated");
      void queryClient.invalidateQueries({ queryKey: ["email-templates"] });
      const record = created as unknown as { id: string };
      void navigate({ to: "/settings/email-templates/$id", params: { id: record.id } });
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection("_emailTemplates").delete(id),
    onSuccess: () => {
      toast.success("Template deleted");
      void queryClient.invalidateQueries({ queryKey: ["email-templates"] });
      setDeleteTarget(null);
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const templates = data ?? [];
  const item = settingsItemFor("/settings/email-templates")!;

  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={
        <Button size="sm" className="gap-1.5" asChild>
          <Link to="/settings/email-templates/$id" params={{ id: "new" }}>
            <Plus className="size-3.5" />
            New template
          </Link>
        </Button>
      }
    >
      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <FileText />
            </EmptyMedia>
            <EmptyTitle>Couldn't load templates</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : isLoading ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 4 }, (_, i) => (
            <Skeleton key={i} className="h-row w-full" />
          ))}
        </div>
      ) : templates.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <FileText />
            </EmptyMedia>
            <EmptyTitle>No templates yet</EmptyTitle>
            <EmptyDescription>Create one to start sending branded, editable email.</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Key</TableHead>
              <TableHead>Name</TableHead>
              <TableHead>Locale</TableHead>
              <TableHead>Editor</TableHead>
              <TableHead>Who can send it</TableHead>
              <TableHead className="w-24 text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {templates.map((template) => (
              <TableRow key={template.id} className="align-top">
                <TableCell className="py-3">
                  <Link
                    to="/settings/email-templates/$id"
                    params={{ id: template.id }}
                    className="font-mono text-sm font-medium hover:underline"
                  >
                    {template.key}
                  </Link>
                </TableCell>
                <TableCell className="py-3 text-sm">{template.name}</TableCell>
                <TableCell className="py-3 text-sm text-muted-foreground">{template.locale || "(default)"}</TableCell>
                <TableCell className="py-3 text-sm capitalize text-muted-foreground">
                  {template.editor || "html"}
                </TableCell>
                <TableCell className="py-3">
                  <SendRuleBadge sendRule={template.sendRule} />
                </TableCell>
                <TableCell className="py-3 text-right">
                  <div className="flex justify-end gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Duplicate ${template.key}`}
                      disabled={duplicate.isPending}
                      onClick={() => duplicate.mutate(template)}
                    >
                      <Copy className="size-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Delete ${template.key}`}
                      onClick={() => setDeleteTarget(template)}
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

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete "{deleteTarget?.key}"?</DialogTitle>
            <DialogDescription>
              Any auth flow or trigger still pointing at this key falls back to the built-in default, if there is one,
              or fails to send. There is no undo.
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
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsPage>
  );
}
