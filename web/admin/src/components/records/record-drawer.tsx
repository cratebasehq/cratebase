import { useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Ban, Check, Copy, ShieldUser, Trash2, UserCog } from "lucide-react";
import type { CollectionModel, RecordModel } from "@cratebase/client";
import { isMultiValue, userFields, type FieldSchema } from "@/lib/field-types";
import { singularize, validateRecordDraft, type FileDraft } from "@/lib/record-validation";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
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
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";
import { RecordFieldInput } from "@/components/records/record-field-input";
import { FileField } from "@/components/records/file-field";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { useRecordMutations } from "@/hooks/use-records";
import { cb, describeFailure, superuserAuth } from "@/lib/api";
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
  open: boolean;
  onOpenChange: (open: boolean) => void;
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

/** A copyable identifier line for the metadata strip. */
function MetaValue({ label, value, mono = true }: { label: string; value: string; mono?: boolean }) {
  const { copy, status } = useCopyToClipboard();
  return (
    <div className="flex min-w-0 items-baseline gap-2">
      <dt className="w-16 shrink-0 text-2xs uppercase tracking-wider text-muted-foreground/70">{label}</dt>
      <dd className="flex min-w-0 items-center gap-1">
        <span className={mono ? "truncate font-mono text-xs" : "truncate text-xs"}>{value}</span>
        <button
          type="button"
          aria-label={`Copy ${label}`}
          onClick={() => void copy(value)}
          className="shrink-0 rounded p-0.5 text-muted-foreground opacity-0 transition-opacity hover:text-foreground focus-visible:opacity-100 group-hover/meta:opacity-100"
        >
          {status === "copied" ? <Check className="size-3" /> : <Copy className="size-3" />}
        </button>
      </dd>
    </div>
  );
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
export function RecordDrawer({ collection, record, open, onOpenChange }: RecordDrawerProps) {
  const isNew = record === null;
  const identityField =
    collection.type === "auth"
      ? ((collection.passwordAuth?.identityFields?.[0] as string | undefined) ?? "email")
      : null;

  const fields = useMemo(() => userFields(collection), [collection]);
  const seed = useMemo(
    () => buildDraft(record, fields, identityField),
    [record, fields, identityField],
  );

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

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSubmitted(true);
    if (pending || invalidCount > 0) return;
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
        toast.error("The server rejected some fields", { description: "They're marked below." });
      } else {
        toast.error(failure.title, { description: failure.serverMessage || failure.detail || undefined });
      }
    }
  }

  return (
    <>
      <Sheet open={open} onOpenChange={requestClose}>
        <SheetContent
          side="right"
          className="gap-0 p-0 data-[side=right]:w-full data-[side=right]:sm:max-w-[560px] data-[side=right]:lg:max-w-[640px]"
        >
          {/* `noValidate` on purpose: an `<input type="url">` with a bad
              value makes the browser cancel the submit and show its own
              tooltip, so React's handler never runs and none of the field
              errors below ever appear. Validation is this form's job. */}
          <form onSubmit={handleSubmit} noValidate className="flex h-full min-h-0 flex-col">
            <SheetHeader className="gap-1">
              <SheetTitle>{isNew ? `New ${singularize(collection.name)}` : "Edit record"}</SheetTitle>
              <SheetDescription>
                {isNew ? `Add a row to ${collection.name}.` : `Editing a row in ${collection.name}.`}
              </SheetDescription>
            </SheetHeader>

            <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-4 pb-4">
              {/* What the server knows about this row, which the old drawer
                  never showed — the id is the thing you paste into a filter. */}
              {record ? (
                <dl className="group/meta flex flex-col gap-1 rounded-lg border border-border bg-surface-sunken/60 px-3 py-2">
                  <MetaValue label="id" value={record.id} />
                  {record.created ? (
                    <MetaValue label="created" value={new Date(record.created).toLocaleString()} />
                  ) : null}
                  {record.updated ? (
                    <MetaValue label="updated" value={new Date(record.updated).toLocaleString()} />
                  ) : null}
                </dl>
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

              {(identityField && fields.length > 0) ? <Separator /> : null}

              {fields.map((field) => {
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
              })}
            </div>

            <SheetFooter className="flex-row items-center gap-2 border-t border-border">
              {record ? (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="gap-1.5 text-destructive hover:bg-destructive/10 hover:text-destructive"
                  onClick={() => setConfirmDelete(true)}
                  disabled={pending}
                >
                  <Trash2 className="size-3.5" />
                  Delete
                </Button>
              ) : null}

              {record && isAuth ? (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="gap-1.5"
                  onClick={() => impersonate.mutate()}
                  disabled={pending || impersonate.isPending}
                >
                  {impersonate.isPending ? <Spinner className="size-3.5" /> : <UserCog className="size-3.5" />}
                  Impersonate
                </Button>
              ) : null}

              {record && isAuth ? (
                banStatus.data ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="gap-1.5"
                    onClick={() => unban.mutate()}
                    disabled={pending || unban.isPending}
                  >
                    {unban.isPending ? <Spinner className="size-3.5" /> : <ShieldUser className="size-3.5" />}
                    Unban
                  </Button>
                ) : (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="gap-1.5 text-destructive hover:bg-destructive/10 hover:text-destructive"
                    onClick={() => setConfirmBan(true)}
                    disabled={pending || ban.isPending}
                  >
                    {ban.isPending ? <Spinner className="size-3.5" /> : <Ban className="size-3.5" />}
                    Ban
                  </Button>
                )
              ) : null}

              {blocked ? (
                <span className="min-w-0 truncate text-xs text-destructive">
                  {invalidCount} {invalidCount === 1 ? "field needs" : "fields need"} attention
                </span>
              ) : null}

              <div className="flex-1" />
              <Button type="button" variant="ghost" size="sm" onClick={() => requestClose(false)} disabled={pending}>
                Cancel
              </Button>
              <Button type="submit" size="sm" disabled={pending || blocked}>
                {pending ? <Spinner /> : null}
                {pending ? "Saving…" : isNew ? "Create record" : "Save changes"}
              </Button>
            </SheetFooter>
          </form>
        </SheetContent>
      </Sheet>

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
