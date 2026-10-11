import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Ban, Check, Copy, CopyPlus, ShieldUser, Trash2, UserCog } from "lucide-react";
import type { CollectionModel, RecordModel } from "@cratebase/client";
import { isMultiValue, managedFields, userFields, type FieldSchema } from "@/lib/field-types";
import { singularize, validateRecordDraft, type FileDraft } from "@/lib/record-validation";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Spinner } from "@/components/ui/spinner";
import {
  DetailBody,
  DetailCallout,
  DetailEmpty,
  DetailList,
  DetailRow,
  DetailSection,
  DetailSheet,
  DetailSheetHeader,
  DetailTabPanel,
  DetailTabs,
  type DetailMenuItem,
  type DetailPager,
  type DetailTabItem,
} from "@/components/layout/detail-sheet";
import { StatusBadge } from "@/components/layout/stat-card";
import { CodeBlock } from "@/components/settings/detail-json";
import { TimeAgo } from "@/components/settings/time-ago";
import { RecordFieldInput } from "@/components/records/record-field-input";
import { Kbd } from "@/components/ui/kbd";
import { FileField } from "@/components/records/file-field";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { useRecordMutations } from "@/hooks/use-records";
import { cb, describeFailure, parseServerDate, superuserAuth } from "@/lib/api";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

interface RecordDrawerProps {
  collection: CollectionModel;
  record: RecordModel | null;
  /** Values to start a new record from ("Duplicate"). Ignored when editing. */
  template?: RecordModel | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The records currently loaded in the grid, in order. Drives the
   * "N of M" pager together with `onNavigate`. */
  siblings?: RecordModel[];
  onNavigate?: (record: RecordModel) => void;
  /** Start a new record from this one. Omit to hide the Duplicate action. */
  onDuplicate?: (record: RecordModel) => void;
}

type Draft = Record<string, unknown>;

/** How long a value has to hold still before its error is allowed on
 * screen. Matches the collection form's inline validation. */
const SETTLE_MS = 450;

/** Field errors plus the two auth-only rules the schema itself can't
 * express (the identity field and, on create, a password). */
function allErrors(
  fields: FieldSchema[],
  values: Draft,
  identityField: string | null,
  isNew: boolean,
): Record<string, string> {
  const errors = validateRecordDraft(fields, values);
  if (identityField) {
    if (String(values[identityField] ?? "").trim().length === 0) {
      errors[identityField] = `A ${identityField} is required`;
    }
    if (isNew && String(values.password ?? "").length < 8) {
      errors.password = "At least 8 characters";
    }
  }
  return errors;
}

/** The form's starting value for one field. `file` fields carry a
 * `{ keep, added }` draft rather than a bare value, because "which of the
 * stored files survive" is part of the edit. */
function initialValue(record: RecordModel | null, field: FieldSchema): unknown {
  if (field.type === "file") {
    const stored = record?.[field.name] as string[] | string | undefined;
    const keep = Array.isArray(stored) ? [...stored] : stored ? [stored] : [];
    return { keep, added: [] } satisfies FileDraft;
  }
  if (!record) {
    if (field.type === "bool") return false;
    if (isMultiValue(field)) return [];
    return null;
  }
  const value = record[field.name];
  // JSON is edited as text; seed it pretty-printed.
  if (field.type === "json" && value !== null && value !== undefined && typeof value !== "string") {
    return JSON.stringify(value, null, 2);
  }
  return value ?? null;
}

function buildDraft(record: RecordModel | null, fields: FieldSchema[], identityField: string | null): Draft {
  const draft: Draft = {};
  for (const field of fields) draft[field.name] = initialValue(record, field);
  if (identityField) draft[identityField] = (record?.[identityField] as string | undefined) ?? "";
  return draft;
}

/** Bool fields worth a status chip when switched on. */
const FLAG_FIELDS = /^(published|active|enabled|archived|featured|public|verified)$/i;

type PendingAction = { kind: "nav"; record: RecordModel } | { kind: "duplicate"; record: RecordModel };

function absoluteTime(value: string): string {
  const d = parseServerDate(value);
  return Number.isNaN(d.getTime()) ? value : d.toLocaleString();
}

function formatSystemValue(value: unknown): string {
  if (value === null || value === undefined || value === "") return "—";
  return typeof value === "object" ? JSON.stringify(value) : String(value);
}

/**
 * The record editor.
 *
 * Everything the audit called out is addressed here: values are validated
 * against the field's own constraints before the request rather than after
 * it, JSON parses as you type, files show what is actually stored and can be
 * removed one at a time, relations are searched server-side instead of
 * picked from the first 100 rows, the record's own identifiers are visible
 * and copyable, and closing with unsaved edits asks first.
 */
export function RecordDrawer({
  collection,
  record,
  template = null,
  open,
  onOpenChange,
  siblings,
  onNavigate,
  onDuplicate,
}: RecordDrawerProps) {
  const isNew = record === null;
  const identityField =
    collection.type === "auth"
      ? ((collection.passwordAuth?.identityFields?.[0] as string | undefined) ?? "email")
      : null;

  const fields = useMemo(() => userFields(collection), [collection]);
  const seed = useMemo(
    () => buildDraft(record ?? template, fields, identityField),
    [record, template, fields, identityField],
  );
  // A form with many fields is easier to scan in labelled sections; one with
  // only a few plain fields is not improved by headings.
  const groups = useMemo(() => {
    const all = [
      { key: "fields", title: "Fields", items: fields.filter((f) => f.type !== "relation" && f.type !== "file") },
      { key: "relations", title: "Relations", items: fields.filter((f) => f.type === "relation") },
      { key: "files", title: "Files", items: fields.filter((f) => f.type === "file") },
    ];
    return all.filter((g) => g.items.length > 0);
  }, [fields]);

  const [values, setValues] = useState<Draft>(seed);
  const [seedKey, setSeedKey] = useState(seed);
  const [serverErrors, setServerErrors] = useState<Record<string, string>>({});
  const [touched, setTouched] = useState<Record<string, boolean>>({});
  const [submitted, setSubmitted] = useState(false);
  const [confirmClose, setConfirmClose] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  // Opening the drawer on a different record (or reopening it) re-seeds the
  // form — adjusted during render so no frame shows the previous record.
  if (seedKey !== seed) {
    setSeedKey(seed);
    setValues(seed);
    setServerErrors({});
    setTouched({});
    setSubmitted(false);
  }

  const [tab, setTab] = useState("fields");
  const [pendingAction, setPendingAction] = useState<PendingAction | null>(null);
  const { copy: copyJson } = useCopyToClipboard({ onCopy: () => toast.success("Copied record as JSON") });

  const { create, update, remove } = useRecordMutations(collection.name);
  const pending = create.isPending || update.isPending;

  // Impersonation and bans only make sense for a real, already-saved auth
  // record — never a brand-new draft, and never a base/view collection.
  const isAuth = collection.type === "auth";
  const queryClient = useQueryClient();
  const [impersonateToken, setImpersonateToken] = useState<string | null>(null);
  const [confirmBan, setConfirmBan] = useState(false);

  const banStatus = useQuery({
    queryKey: ["ban-status", collection.id, record?.id],
    queryFn: async () => {
      const list = await cb
        .collection("_bans")
        .list({ filter: `collectionRef = "${collection.id}" && recordRef = "${record!.id}"`, perPage: 1 });
      return list.items[0] ?? null;
    },
    enabled: isAuth && record !== null,
    staleTime: 10_000,
  });

  const impersonate = useMutation({
    mutationFn: () => superuserAuth.admin.impersonate(collection.name, record!.id),
    onSuccess: (ns) => setImpersonateToken(ns.token),
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.serverMessage || failure.detail || undefined });
    },
  });

  const ban = useMutation({
    mutationFn: () => superuserAuth.admin.ban(collection.name, record!.id),
    onSuccess: () => {
      toast.success("Record banned", { description: "Every live session for it was revoked." });
      void queryClient.invalidateQueries({ queryKey: ["ban-status", collection.id, record?.id] });
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.serverMessage || failure.detail || undefined });
    },
  });

  const unban = useMutation({
    mutationFn: () => superuserAuth.admin.unban(collection.name, record!.id),
    onSuccess: () => {
      toast.success("Ban lifted");
      void queryClient.invalidateQueries({ queryKey: ["ban-status", collection.id, record?.id] });
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.serverMessage || failure.detail || undefined });
    },
  });

  const totpStatus = useQuery({
    queryKey: ["totp-status", collection.id, record?.id],
    queryFn: async () => {
      const list = await cb
        .collection("_totps")
        .list({ filter: `collectionRef = "${collection.id}" && recordRef = "${record!.id}"`, perPage: 1 });
      return (list.items[0] as (RecordModel & { confirmed?: boolean }) | undefined) ?? null;
    },
    enabled: isAuth && record !== null,
    staleTime: 10_000,
  });

  const resetTotp = useMutation({
    mutationFn: () => cb.collection("_totps").delete(totpStatus.data!.id),
    onSuccess: () => {
      toast.success("TOTP reset", { description: "The next login will not ask for a second factor until they set it up again." });
      void queryClient.invalidateQueries({ queryKey: ["totp-status", collection.id, record?.id] });
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.serverMessage || failure.detail || undefined });
    },
  });

  const linkedAccounts = useQuery({
    queryKey: ["linked-accounts", collection.id, record?.id],
    queryFn: () => superuserAuth.admin.listExternalAuths(collection.name, record!.id),
    enabled: isAuth && record !== null,
    staleTime: 10_000,
  });

  const unlinkAccount = useMutation({
    mutationFn: (provider: string) => superuserAuth.admin.unlinkExternalAuth(collection.name, record!.id, provider),
    onSuccess: () => {
      toast.success("Provider unlinked");
      void queryClient.invalidateQueries({ queryKey: ["linked-accounts", collection.id, record?.id] });
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.serverMessage || failure.detail || undefined });
    },
  });

  /** Errors as of this keystroke — what gates Save and drives the count. */
  const clientErrors = useMemo(
    () => allErrors(fields, values, identityField, isNew),
    [fields, values, identityField, isNew],
  );

  /**
   * Errors as of ~half a second ago, which is what actually gets *shown*.
   *
   * Validating on blur would be tidier, but a `focusout` inside a Radix
   * sheet never reaches React's delegated handler, so a blur-gated error
   * simply never appears. Settling the message instead means it arrives
   * when you pause rather than on every keystroke of a value that is only
   * briefly invalid — the same rule the collection-name field uses.
   */
  const [settled, setSettled] = useState<Draft>(values);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(values), SETTLE_MS);
    return () => clearTimeout(timer);
  }, [values]);
  const settledErrors = useMemo(
    () => allErrors(fields, settled, identityField, isNew),
    [fields, settled, identityField, isNew],
  );

  const dirty = JSON.stringify(serialisableDraft(values)) !== JSON.stringify(serialisableDraft(seed));
  const invalidCount = Object.keys(clientErrors).length;
  // Save stays live until something on screen explains why it wouldn't
  // work — a button disabled before you have touched anything just looks
  // broken, and a record can be invalid the moment it loads if its schema
  // was tightened after the row was written.
  const blocked = invalidCount > 0 && (submitted || Object.keys(touched).length > 0);

  /** Show a field's error once Save was tried, or once the value it was
   * given has settled — never while it is still mid-edit. */
  function errorFor(name: string): string | undefined {
    if (serverErrors[name]) return serverErrors[name];
    if (submitted) return clientErrors[name];
    if (!touched[name]) return undefined;
    // Only a settled error that is still true of the live value, so a
    // fixed field clears immediately instead of after the delay.
    return settledErrors[name] && clientErrors[name] ? clientErrors[name] : undefined;
  }

  function setValue(name: string, value: unknown) {
    setValues((v) => ({ ...v, [name]: value }));
    setTouched((t) => (t[name] ? t : { ...t, [name]: true }));
    setServerErrors(({ [name]: _dropped, ...rest }) => rest);
  }

  function requestClose(next: boolean) {
    if (!next && dirty && !pending) {
      setConfirmClose(true);
      return;
    }
    onOpenChange(next);
  }

  /**
   * Turn the draft into what the API takes.
   *
   * Files force multipart, and the server's own `field+` / `field-`
   * modifiers are how individual files are added to and removed from an
   * existing record — sending the field plainly would replace the lot.
   */
  function buildPayload(): Record<string, unknown> | FormData {
    const plain: Record<string, unknown> = {};
    for (const field of fields) {
      const value = values[field.name];
      if (field.type === "file" || field.type === "autodate") continue;
      if (field.type === "json") {
        plain[field.name] = typeof value === "string" && value.trim() ? JSON.parse(value) : null;
        continue;
      }
      if (field.type === "password" && !value) continue;
      plain[field.name] = value;
    }
    if (identityField) plain[identityField] = values[identityField];
    if (identityField && values.password) {
      plain.password = values.password;
      plain.passwordConfirm = values.password;
    }

    const fileFields = fields.filter((f) => f.type === "file");
    const drafts = fileFields.map((f) => [f, (values[f.name] ?? { keep: [], added: [] }) as FileDraft] as const);
    const hasUploads = drafts.some(([, d]) => d.added.length > 0);
    const removals = drafts.flatMap(([field, draft]) => {
      const stored = (record?.[field.name] as string[] | string | undefined) ?? [];
      const storedNames = Array.isArray(stored) ? stored : stored ? [stored] : [];
      const gone = storedNames.filter((name) => !draft.keep.includes(name));
      return gone.map((name) => [field.name, name] as const);
    });

    if (!hasUploads) {
      // No multipart needed: removals go as `field-` in the JSON body.
      for (const [fieldName, filename] of removals) {
        const key = `${fieldName}-`;
        const list = (plain[key] as string[] | undefined) ?? [];
        plain[key] = [...list, filename];
      }
      return plain;
    }

    const form = new FormData();
    for (const [key, value] of Object.entries(plain)) {
      if (value === null || value === undefined) {
        form.append(key, "");
      } else if (Array.isArray(value)) {
        if (value.length === 0) form.append(key, "");
        else for (const item of value) form.append(key, typeof item === "string" ? item : JSON.stringify(item));
      } else if (typeof value === "object") {
        form.append(key, JSON.stringify(value));
      } else {
        form.append(key, String(value));
      }
    }
    for (const [fieldName, filename] of removals) form.append(`${fieldName}-`, filename);
    for (const [field, draft] of drafts) {
      for (const file of draft.added) {
        // On an existing record `+` appends to what's kept; on a new one
        // there is nothing to append to.
        form.append(isNew || !isMultiValue(field) ? field.name : `${field.name}+`, file);
      }
    }
    return form;
  }

  /** The tab holding a field, so a rejected Save lands on the problem. */
  function tabOf(name: string): string {
    const type = fields.find((f) => f.name === name)?.type;
    return type === "relation" ? "relations" : type === "file" ? "files" : "fields";
  }

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSubmitted(true);
    if (pending) return;
    if (invalidCount > 0) {
      setTab(tabOf(Object.keys(clientErrors)[0]));
      return;
    }
    setServerErrors({});
    try {
      if (isNew) {
        await create.mutateAsync(buildPayload());
        toast.success("Record created");
      } else {
        await update.mutateAsync({ id: record.id, data: buildPayload() });
        toast.success("Record updated");
      }
      onOpenChange(false);
    } catch (error) {
      const failure = describeFailure(error);
      if (Object.keys(failure.fields).length > 0) {
        setServerErrors(failure.fields);
        setTab(tabOf(Object.keys(failure.fields)[0]));
        toast.error("The server rejected some fields", { description: "They're marked below." });
      } else {
        toast.error(failure.title, { description: failure.serverMessage || failure.detail || undefined });
      }
    }
  }

  /** Leaving this record (pager, duplicate) with unsaved edits asks first. */
  function runAction(action: PendingAction) {
    if (action.kind === "nav") onNavigate?.(action.record);
    else onDuplicate?.(action.record);
  }
  function guarded(action: PendingAction) {
    if (dirty && !pending) setPendingAction(action);
    else runAction(action);
  }

  // ⌘/Ctrl+Enter saves from anywhere in the sheet — the Save button lives in
  // the header, outside the form, so a form-level key handler would miss it.
  const formRef = useRef<HTMLFormElement>(null);
  useEffect(() => {
    if (!open) return;
    function onKey(event: KeyboardEvent) {
      if (event.key !== "Enter" || !(event.metaKey || event.ctrlKey)) return;
      if ((event.target as HTMLElement | null)?.closest('[role="alertdialog"]')) return;
      event.preventDefault();
      formRef.current?.requestSubmit();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  function renderField(field: FieldSchema) {
    const error = errorFor(field.name);
    return (
      <div
        key={field.id || field.name}
        className="flex flex-col gap-1.5"
        onBlur={() => setTouched((t) => ({ ...t, [field.name]: true }))}
      >
        <div className="flex items-baseline justify-between gap-2">
          <Label className="font-mono">
            {field.name}
            {field.required ? <span className="text-destructive"> *</span> : null}
          </Label>
          <span className="shrink-0 text-2xs text-muted-foreground/70">{field.type}</span>
        </div>

        {field.type === "file" ? (
          <FileField
            field={field}
            record={record}
            value={(values[field.name] ?? { keep: [], added: [] }) as FileDraft}
            onChange={(next) => setValue(field.name, next)}
            invalid={Boolean(error)}
          />
        ) : (
          <RecordFieldInput
            field={field}
            value={values[field.name]}
            onChange={(value) => setValue(field.name, value)}
            error={error}
          />
        )}
        <FieldError message={error} />
      </div>
    );
  }

  /* ---- Detail-sheet chrome ------------------------------------------------ */

  const plainFields = groups.find((g) => g.key === "fields")?.items ?? [];
  const relationFields = groups.find((g) => g.key === "relations")?.items ?? [];
  const fileFields = groups.find((g) => g.key === "files")?.items ?? [];
  const fileCount = fileFields.reduce((n, f) => {
    const d = (values[f.name] ?? { keep: [], added: [] }) as FileDraft;
    return n + d.keep.length + d.added.length;
  }, 0);

  const tabs: DetailTabItem[] = [
    { value: "fields", label: "Fields" },
    ...(relationFields.length > 0 ? [{ value: "relations", label: "Relations", count: relationFields.length }] : []),
    ...(fileFields.length > 0 ? [{ value: "files", label: "Files", count: fileCount }] : []),
    ...(record ? [{ value: "json", label: "JSON" }] : []),
    ...(isAuth && record ? [{ value: "account", label: "Account" }] : []),
    ...(record ? [{ value: "meta", label: "Meta" }] : []),
  ];
  const activeTab = tabs.some((t) => t.value === tab) ? tab : "fields";

  const presentable = fields.find((f) => f.presentable)?.name;
  const titleSource = [presentable, identityField, "title", "name", "email"].find(
    (key) => key && ["string", "number"].includes(typeof record?.[key]) && String(record?.[key]).trim() !== "",
  );
  const title = isNew
    ? template
      ? `Duplicate ${singularize(collection.name)}`
      : `New ${singularize(collection.name)}`
    : titleSource
      ? String(record[titleSource])
      : record.id;

  const statusChips = (
    <>
      {dirty ? <StatusBadge tone="warning">Unsaved</StatusBadge> : null}
      {record && isAuth && "verified" in record ? (
        <StatusBadge tone={record.verified ? "success" : "neutral"}>
          {record.verified ? "Verified" : "Unverified"}
        </StatusBadge>
      ) : null}
      {banStatus.data ? <StatusBadge tone="danger">Banned</StatusBadge> : null}
      {record
        ? fields
            .filter((f) => f.type === "bool" && FLAG_FIELDS.test(f.name) && record[f.name] === true)
            .map((f) => (
              <StatusBadge key={f.name} tone="success">
                {f.name.charAt(0).toUpperCase() + f.name.slice(1)}
              </StatusBadge>
            ))
        : null}
    </>
  );

  const metaLine: React.ReactNode[] = record
    ? [
        <span key="c" className="font-mono">{collection.name}</span>,
        record.created ? <span key="cr">created <TimeAgo value={record.created} /></span> : null,
        record.updated ? <span key="up">updated <TimeAgo value={record.updated} /></span> : null,
      ]
    : [
        <span key="c" className="font-mono">{collection.name}</span>,
        template ? "Files and passwords are not copied" : "Not saved yet",
      ];

  const siblingIndex = record && siblings ? siblings.findIndex((r) => r.id === record.id) : -1;
  const pager: DetailPager | undefined =
    siblings && onNavigate && siblingIndex >= 0 && siblings.length > 1
      ? {
          index: siblingIndex + 1,
          total: siblings.length,
          hasPrev: siblingIndex > 0,
          hasNext: siblingIndex < siblings.length - 1,
          onPrev: () => guarded({ kind: "nav", record: siblings[siblingIndex - 1] }),
          onNext: () => guarded({ kind: "nav", record: siblings[siblingIndex + 1] }),
        }
      : undefined;

  const menu: DetailMenuItem[] = record
    ? [
        {
          label: "Copy JSON",
          icon: <Copy />,
          onSelect: () => void copyJson(JSON.stringify(record, null, 2)),
        },
        ...(isAuth
          ? [
              { label: "Impersonate", icon: <UserCog />, onSelect: () => !impersonate.isPending && impersonate.mutate() },
              banStatus.data
                ? { label: "Unban", icon: <ShieldUser />, onSelect: () => unban.mutate() }
                : { label: "Ban…", icon: <Ban />, destructive: true, onSelect: () => setConfirmBan(true) },
            ]
          : []),
        { label: "Delete…", icon: <Trash2 />, destructive: true, onSelect: () => setConfirmDelete(true) },
      ]
    : [];

  // The managed columns beyond id/created/updated (auth flags and the like),
  // minus anything secret. Reachable from the Meta tab.
  const systemRows: [string, string][] = record
    ? managedFields(collection)
        .filter((f) => !["id", "created", "updated", "password", "tokenKey"].includes(f.name))
        .map((f) => [f.name, formatSystemValue(record[f.name])] as [string, string])
    : [];

  return (
    <>
      <DetailSheet open={open} onOpenChange={requestClose}>
        <DetailSheetHeader
          id={record?.id}
          pager={pager}
          menu={menu}
          title={title}
          description={isNew ? `Draft record in ${collection.name}` : `Editing ${record.id} in ${collection.name}`}
          status={statusChips}
          meta={metaLine}
          actions={
            <>
              <Button type="submit" form="record-form" size="sm" disabled={pending || blocked}>
                {pending ? <Spinner /> : null}
                {pending ? "Saving…" : isNew ? "Create record" : "Save changes"}
              </Button>
              {record && onDuplicate ? (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={pending}
                  onClick={() => guarded({ kind: "duplicate", record })}
                >
                  <CopyPlus />
                  Duplicate
                </Button>
              ) : null}
              {blocked ? (
                <span className="min-w-0 truncate text-xs text-destructive">
                  {invalidCount} {invalidCount === 1 ? "field needs" : "fields need"} attention
                </span>
              ) : (
                <span className="ml-auto hidden items-center gap-1 text-2xs text-muted-foreground md:inline-flex">
                  <Kbd>⌘</Kbd>
                  <Kbd>↵</Kbd>
                  to save
                </span>
              )}
            </>
          }
        />

        {/* `noValidate` on purpose: an `<input type="url">` with a bad
            value makes the browser cancel the submit and show its own
            tooltip, so React's handler never runs and none of the field
            errors below ever appear. Validation is this form's job. The
            header's Save button targets this form by id. */}
        <form
          id="record-form"
          ref={formRef}
          onSubmit={handleSubmit}
          noValidate
          className="flex min-h-0 flex-1 flex-col"
        >
          <DetailTabs tabs={tabs} value={activeTab} onValueChange={setTab}>
            <DetailTabPanel value="fields">
              <DetailBody className="gap-4">
                {banStatus.data ? (
                  <DetailCallout
                    tone="danger"
                    icon={<Ban />}
                    title="Banned"
                    action={
                      <Button
                        type="button"
                        variant="outline"
                        size="xs"
                        onClick={() => unban.mutate()}
                        disabled={pending || unban.isPending}
                      >
                        {unban.isPending ? <Spinner className="size-3.5" /> : null}
                        Unban
                      </Button>
                    }
                  >
                    Every live session was revoked and sign-in is blocked until the ban is lifted.
                  </DetailCallout>
                ) : null}

                {identityField ? (
                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor="record-identity">{identityField}</Label>
                    <Input
                      id="record-identity"
                      type={identityField === "email" ? "email" : "text"}
                      value={(values[identityField] as string) ?? ""}
                      onChange={(e) => setValue(identityField, e.target.value)}
                      onBlur={() => setTouched((t) => ({ ...t, [identityField]: true }))}
                      aria-invalid={errorFor(identityField) ? true : undefined}
                      className="h-control-md"
                    />
                    <FieldError message={errorFor(identityField)} />
                  </div>
                ) : null}

                {identityField ? (
                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor="record-password">{isNew ? "Password" : "New password"}</Label>
                    <Input
                      id="record-password"
                      type="password"
                      value={(values.password as string) ?? ""}
                      onChange={(e) => setValue("password", e.target.value)}
                      onBlur={() => setTouched((t) => ({ ...t, password: true }))}
                      placeholder={isNew ? "At least 8 characters" : "Leave blank to keep the current one"}
                      aria-invalid={errorFor("password") ? true : undefined}
                      className="h-control-md"
                    />
                    <FieldError message={errorFor("password")} />
                  </div>
                ) : null}

                {plainFields.map(renderField)}
                {!identityField && plainFields.length === 0 ? (
                  <DetailEmpty
                    title="No editable fields"
                    description="This collection only has relation or file fields, or none yet."
                  />
                ) : null}
              </DetailBody>
            </DetailTabPanel>

            {relationFields.length > 0 ? (
              <DetailTabPanel value="relations">
                <DetailBody className="gap-4">{relationFields.map(renderField)}</DetailBody>
              </DetailTabPanel>
            ) : null}

            {fileFields.length > 0 ? (
              <DetailTabPanel value="files">
                <DetailBody className="gap-4">{fileFields.map(renderField)}</DetailBody>
              </DetailTabPanel>
            ) : null}

            {record ? (
              <DetailTabPanel value="json">
                <DetailBody>
                  <CodeBlock
                    title={dirty ? "Saved record (unsaved edits not included)" : "Saved record"}
                    json={JSON.stringify(record, null, 2)}
                    copyLabel="Copy JSON"
                  />
                </DetailBody>
              </DetailTabPanel>
            ) : null}

            {isAuth && record ? (
              <DetailTabPanel value="account">
                <DetailBody>
                  <DetailSection title="Two-factor authentication">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-sm text-muted-foreground">
                        {totpStatus.isLoading
                          ? "Checking…"
                          : totpStatus.data?.confirmed
                            ? "TOTP is enabled for this account."
                            : "TOTP is not enabled."}
                      </span>
                      {totpStatus.data?.confirmed ? (
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="gap-1.5 text-destructive hover:bg-destructive/10 hover:text-destructive"
                          onClick={() => resetTotp.mutate()}
                          disabled={resetTotp.isPending}
                        >
                          {resetTotp.isPending ? <Spinner className="size-3.5" /> : null}
                          Reset TOTP
                        </Button>
                      ) : null}
                    </div>
                  </DetailSection>
                  <DetailSection title="Linked accounts">
                    {linkedAccounts.isLoading ? (
                      <span className="text-sm text-muted-foreground">Checking…</span>
                    ) : linkedAccounts.data && linkedAccounts.data.length > 0 ? (
                      <ul className="m-0 flex list-none flex-col p-0">
                        {linkedAccounts.data.map((row) => (
                          <li key={row.id} className="flex min-h-8 items-center justify-between gap-2 text-sm">
                            <span className="font-mono">{row.provider}</span>
                            <Button
                              type="button"
                              variant="ghost"
                              size="sm"
                              className="gap-1.5 text-destructive hover:bg-destructive/10 hover:text-destructive"
                              onClick={() => unlinkAccount.mutate(row.provider)}
                              disabled={unlinkAccount.isPending}
                            >
                              Unlink
                            </Button>
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <span className="text-sm text-muted-foreground">No linked OAuth2 providers.</span>
                    )}
                  </DetailSection>
                </DetailBody>
              </DetailTabPanel>
            ) : null}

            {record ? (
              <DetailTabPanel value="meta">
                <DetailBody>
                  <DetailSection title="Record">
                    <DetailList>
                      <DetailRow label="id">
                        <span className="font-mono break-all">{record.id}</span>
                      </DetailRow>
                      <DetailRow label="collectionName">
                        <span className="font-mono">{record.collectionName ?? collection.name}</span>
                      </DetailRow>
                      <DetailRow label="collectionId">
                        <span className="font-mono break-all">{record.collectionId ?? collection.id}</span>
                      </DetailRow>
                    </DetailList>
                  </DetailSection>
                  <DetailSection title="Timing">
                    <DetailList>
                      <DetailRow label="created" hint={record.created ? absoluteTime(record.created) : undefined}>
                        <TimeAgo value={record.created} />
                      </DetailRow>
                      <DetailRow label="updated" hint={record.updated ? absoluteTime(record.updated) : undefined}>
                        <TimeAgo value={record.updated} />
                      </DetailRow>
                    </DetailList>
                  </DetailSection>
                  {systemRows.length > 0 ? (
                    <DetailSection title="System fields">
                      <DetailList>
                        {systemRows.map(([key, value]) => (
                          <DetailRow key={key} label={key}>
                            <span className="font-mono break-all">{value}</span>
                          </DetailRow>
                        ))}
                      </DetailList>
                    </DetailSection>
                  ) : null}
                </DetailBody>
              </DetailTabPanel>
            ) : null}
          </DetailTabs>
        </form>
      </DetailSheet>

      <AlertDialog open={confirmClose} onOpenChange={setConfirmClose}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard this edit?</AlertDialogTitle>
            <AlertDialogDescription>
              {isNew
                ? "This record hasn't been created yet — closing loses what you've filled in."
                : "Your changes to this record haven't been saved. Closing loses them."}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setConfirmClose(false);
                onOpenChange(false);
              }}
            >
              Discard
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={pendingAction !== null} onOpenChange={(next) => !next && setPendingAction(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard unsaved changes?</AlertDialogTitle>
            <AlertDialogDescription>
              {pendingAction?.kind === "duplicate"
                ? "Duplicating starts from the saved record, so your edits to this one are lost."
                : "Moving to another record loses the edits you haven't saved."}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                const action = pendingAction;
                setPendingAction(null);
                if (action) runAction(action);
              }}
            >
              Discard
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={confirmDelete} onOpenChange={setConfirmDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete this record?</AlertDialogTitle>
            <AlertDialogDescription>
              <span className="font-mono">{record?.id}</span> will be permanently removed from{" "}
              <span className="font-mono">{collection.name}</span>, along with any files attached to it. This cannot
              be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setConfirmDelete(false);
                if (!record) return;
                remove.mutate(record.id, {
                  onSuccess: () => {
                    toast.success("Record deleted");
                    onOpenChange(false);
                  },
                  onError: (error) => {
                    const failure = describeFailure(error);
                    toast.error(failure.title, { description: failure.detail || undefined });
                  },
                });
              }}
            >
              Delete record
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={confirmBan} onOpenChange={setConfirmBan}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Ban this record?</AlertDialogTitle>
            <AlertDialogDescription>
              <span className="font-mono">{record?.id}</span> will be signed out of every live session and unable to
              authenticate again in <span className="font-mono">{collection.name}</span> until unbanned. This takes
              effect immediately.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setConfirmBan(false);
                ban.mutate();
              }}
            >
              Ban record
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {impersonateToken ? (
        <ImpersonationTokenDialog token={impersonateToken} onOpenChange={(open) => !open && setImpersonateToken(null)} />
      ) : null}
    </>
  );
}

/** Shows a freshly minted impersonation token exactly once — the same
 * one-time-secret convention as an API key's reveal dialog and a
 * webhook's signing secret, and for the same reason: the server never
 * returns it a second time (`crate::routes::auth::impersonate` mints it
 * fresh per call and stores only a hash in `_sessions`). Non-refreshable
 * and short-lived by design — a superuser's one-shot loan of a session,
 * not a credential the impersonated record can extend on its own. */
function ImpersonationTokenDialog({ token, onOpenChange }: { token: string; onOpenChange: (open: boolean) => void }) {
  const { copy, status } = useCopyToClipboard();

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Impersonation token</DialogTitle>
          <DialogDescription>
            A bearer token signed in as this record. It is not refreshable and expires with the collection's own
            auth token duration — shown once; the server keeps only a hash of it after this dialog closes.
          </DialogDescription>
        </DialogHeader>

        <div className="relative">
          <pre className="overflow-x-auto rounded-lg border border-border bg-surface-sunken px-3 py-2.5 font-mono text-xs leading-relaxed text-foreground">
            {token}
          </pre>
          <button
            type="button"
            onClick={() => void copy(token)}
            aria-label={status === "copied" ? "Copied" : "Copy token"}
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

function FieldError({ message }: { message?: string }) {
  if (!message) return null;
  return <p className="text-xs text-destructive">{message}</p>;
}

/** `File` objects don't survive `JSON.stringify`, so the dirty check
 * compares their identity by name and size instead. */
function serialisableDraft(draft: Draft): unknown {
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(draft)) {
    if (value && typeof value === "object" && "added" in value && "keep" in value) {
      const file = value as FileDraft;
      out[key] = { keep: file.keep, added: file.added.map((f) => `${f.name}:${f.size}`) };
    } else {
      out[key] = value;
    }
  }
  return out;
}
