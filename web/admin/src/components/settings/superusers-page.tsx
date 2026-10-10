import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Check, Circle, KeyRound, Plus, ShieldUser, Trash2 } from "lucide-react";
import type { RecordModel } from "@cratebase/client";
import { cb, currentSuperuser, describeFailure } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
import { cn } from "@/lib/utils";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { StatusBadge } from "@/components/layout/stat-card";
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
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { SettingsPage } from "@/components/settings/settings-form";
import { TimeAgo } from "@/components/settings/time-ago";
import { TypedConfirmDialog } from "@/components/settings/typed-confirm-dialog";

const COLLECTION = "_superusers";
const MIN_PASSWORD = 8;
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
/** `_superusers.role` values (`cratebase_core::SUPERUSER_ROLE_OWNER`/
 * `SUPERUSER_ROLE_ADMIN` on the server). Owners can manage every other
 * superuser account; admins have full dashboard/API access otherwise. */
const OWNER = "owner";
const ADMIN = "admin";

const HEAD = "h-control-md text-xs font-medium text-muted-foreground";
const CELL = "py-1.5";

/** Toast description for a failed create/update: field-level server errors
 * first, otherwise the server's sentence. */
function failureDescription(error: unknown): { title: string; description: string } {
  const failure = describeFailure(error);
  const fields = Object.entries(failure.fields).map(([k, v]) => `${k}: ${v}`);
  return {
    title: failure.title,
    description: fields.length > 0 ? fields.join("; ") : failure.serverMessage || failure.detail,
  };
}

/**
 * Superusers are ordinary auth records in `_superusers` (there is no
 * separate admins API), so this is a small CRUD over that collection —
 * but it is the one collection you cannot safely browse in the generic
 * records grid, because locking yourself out of it has no undo from the UI.
 */
export function SuperusersPage() {
  const queryClient = useQueryClient();
  const me = currentSuperuser();
  const [creating, setCreating] = useState(false);
  const [deleting, setDeleting] = useState<RecordModel | null>(null);
  const [resetting, setResetting] = useState<RecordModel | null>(null);

  const superusers = useQuery({
    queryKey: ["superusers"],
    queryFn: () => cb.collection(COLLECTION).fullList({ sort: "created" }),
  });

  function invalidate() {
    return queryClient.invalidateQueries({ queryKey: ["superusers"] });
  }

  const remove = useMutation({
    mutationFn: (record: RecordModel) => cb.collection(COLLECTION).delete(record.id),
    onSuccess: async (_result, record) => {
      setDeleting(null);
      await invalidate();
      toast.success(`Removed ${String(record.email ?? record.id)}`);
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error("Couldn't remove the superuser", { description: failure.serverMessage || failure.detail });
    },
  });

  const changeRole = useMutation({
    mutationFn: ({ id, role }: { id: string; role: string; email: string }) =>
      cb.collection(COLLECTION).update(id, { role }),
    onSuccess: async (_result, vars) => {
      await invalidate();
      toast.success(`${vars.email} is now ${vars.role === OWNER ? "an owner" : "an admin"}`);
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error("Couldn't change the role", { description: failure.serverMessage || failure.detail });
    },
  });

  const rows = superusers.data ?? [];
  // Only an owner may create another superuser, change anyone's role, or
  // delete an account — the server enforces exactly this
  // (`RequireOwner`/`routes::records`'s `_superusers`-specific write
  // guards); the UI mirrors it so it never offers a control the API
  // would reject. `myRole` comes from the freshly fetched list rather
  // than the (possibly stale) auth-store record, so a role change made
  // elsewhere in the same session is reflected immediately.
  const myRole = rows.find((r) => r.id === me?.id)?.role;
  const isOwner = myRole === OWNER;
  const ownerCount = rows.filter((r) => r.role === OWNER).length;

  const item = settingsItemFor("/settings/superusers")!;
  return (
    <SettingsPage
      title={item.label}
      description={item.description}
      width="wide"
      action={
        isOwner ? (
          <Button size="sm" onClick={() => setCreating(true)}>
            <Plus />
            New superuser
          </Button>
        ) : undefined
      }
    >
      {superusers.isPending ? (
        <ListSkeleton rows={3} />
      ) : superusers.isError ? (
        <ErrorState
          compact
          title="Couldn't load superusers"
          error={describeFailure(superusers.error).detail}
          onRetry={() => void superusers.refetch()}
        />
      ) : rows.length === 0 ? (
        <EmptyState
          compact
          icon={<ShieldUser />}
          title="No superusers"
          description="Create one from the command line with cratebase superuser create."
        />
      ) : (
        <div className="overflow-hidden rounded-lg border border-border bg-card">
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className={cn(HEAD, "pl-3")}>Email</TableHead>
                <TableHead className={HEAD}>Role</TableHead>
                <TableHead className={HEAD}>Created</TableHead>
                <TableHead className={cn(HEAD, "pr-3 text-right")}>
                  <span className="sr-only">Actions</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((row) => {
                const isMe = row.id === me?.id;
                const email = String(row.email ?? row.id);
                const role = typeof row.role === "string" && row.role ? row.role : ADMIN;
                // The lockout the server refuses to let happen: the last
                // owner can't be re-roled or removed, by anyone (in
                // practice always the viewer's own row — nobody else could
                // be the sole owner while also being able to act here).
                const isSoleOwner = role === OWNER && ownerCount <= 1;
                const removeBlocked = isMe
                  ? "You can't remove the account you're signed in with"
                  : isSoleOwner
                    ? "The last remaining owner can't be removed"
                    : undefined;
                return (
                  <TableRow key={row.id}>
                    <TableCell className={cn(CELL, "pl-3")}>
                      <span className="flex items-center gap-2">
                        <ShieldUser aria-hidden className="size-4 shrink-0 text-muted-foreground" />
                        <span className="min-w-0 truncate font-mono text-xs">{email}</span>
                        {isMe ? <StatusBadge tone="brand">You</StatusBadge> : null}
                      </span>
                    </TableCell>
                    <TableCell className={CELL}>
                      {isOwner ? (
                        <Select
                          value={role}
                          disabled={isSoleOwner || changeRole.isPending}
                          onValueChange={(next) => changeRole.mutate({ id: row.id, role: next, email })}
                        >
                          <SelectTrigger
                            size="sm"
                            aria-label={`Role for ${email}`}
                            className="h-control-sm w-28"
                            title={isSoleOwner ? "The last remaining owner can't be re-roled" : undefined}
                          >
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value={OWNER}>Owner</SelectItem>
                            <SelectItem value={ADMIN}>Admin</SelectItem>
                          </SelectContent>
                        </Select>
                      ) : (
                        <StatusBadge tone={role === OWNER ? "info" : "neutral"} className="capitalize">
                          {role}
                        </StatusBadge>
                      )}
                    </TableCell>
                    <TableCell className={cn(CELL, "text-xs text-muted-foreground")}>
                      <TimeAgo value={typeof row.created === "string" ? row.created : null} />
                    </TableCell>
                    <TableCell className={cn(CELL, "pr-3")}>
                      <div className="flex items-center justify-end gap-1">
                        {isOwner || isMe ? (
                          <Button variant="ghost" size="sm" onClick={() => setResetting(row)}>
                            <KeyRound />
                            Password
                            <span className="sr-only"> for {email}</span>
                          </Button>
                        ) : null}
                        {isOwner ? (
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            aria-label={`Remove ${email}`}
                            className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                            disabled={removeBlocked !== undefined}
                            title={removeBlocked ?? "Remove superuser"}
                            onClick={() => setDeleting(row)}
                          >
                            <Trash2 />
                          </Button>
                        ) : null}
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      )}

      <div className="flex flex-col gap-1 text-xs text-pretty text-muted-foreground">
        {!isOwner && !superusers.isPending && !superusers.isError ? (
          <p>Only owners can add or remove superusers and change roles. You can still change your own password.</p>
        ) : null}
        <p>
          A superuser can also be created from the command line with{" "}
          <code className="font-mono">cratebase superuser create &lt;email&gt; &lt;password&gt;</code> — the way back
          in if you lock yourself out. Command-line superusers are always created as owners.
        </p>
      </div>

      <CreateSuperuserDialog
        open={creating}
        onOpenChange={setCreating}
        onCreated={() => {
          void invalidate();
          setCreating(false);
        }}
      />

      <PasswordDialog record={resetting} onOpenChange={(open) => !open && setResetting(null)} />

      <TypedConfirmDialog
        open={deleting !== null}
        onOpenChange={(open) => !open && !remove.isPending && setDeleting(null)}
        title="Remove this superuser?"
        description={
          <>
            <span className="font-mono">{String(deleting?.email ?? "")}</span> loses access to this dashboard and the
            whole API immediately. Their audit-log entries stay, attributed to a removed superuser.
          </>
        }
        confirmValue={String(deleting?.email ?? "")}
        confirmLabel="Remove superuser"
        pendingLabel="Removing…"
        pending={remove.isPending}
        onConfirm={() => deleting && remove.mutate(deleting)}
      />
    </SettingsPage>
  );
}

/** One live rule line under a password field: neutral until the user has
 * typed, then green when met and red when not. Never colour-only — the icon
 * and the sentence carry the state too. */
function Rule({ ok, active, children }: { ok: boolean; active: boolean; children: React.ReactNode }) {
  return (
    <li
      className={cn(
        "flex items-center gap-1.5 text-xs",
        !active ? "text-muted-foreground" : ok ? "text-success" : "text-destructive",
      )}
    >
      {ok ? <Check aria-hidden className="size-3" /> : <Circle aria-hidden className="size-3" />}
      {children}
      <span className="sr-only">{!active ? "" : ok ? " — met" : " — not met"}</span>
    </li>
  );
}

/** Password + confirmation with inline validation. Shared by the create and
 * change-password dialogs so the two can never disagree about the rules. */
function PasswordFields({
  idPrefix,
  password,
  confirm,
  onPassword,
  onConfirm,
  passwordLabel,
}: {
  idPrefix: string;
  password: string;
  confirm: string;
  onPassword: (value: string) => void;
  onConfirm: (value: string) => void;
  passwordLabel: string;
}) {
  const longEnough = password.length >= MIN_PASSWORD;
  const matches = password.length > 0 && password === confirm;
  const lengthBad = password.length > 0 && !longEnough;
  const mismatch = confirm.length > 0 && password !== confirm;
  return (
    <>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${idPrefix}-password`}>{passwordLabel}</Label>
        <Input
          id={`${idPrefix}-password`}
          type="password"
          autoComplete="new-password"
          value={password}
          onChange={(e) => onPassword(e.target.value)}
          aria-invalid={lengthBad ? true : undefined}
          aria-describedby={`${idPrefix}-rules`}
          className="h-control-md"
        />
        <ul id={`${idPrefix}-rules`} className="flex flex-col gap-0.5">
          <Rule ok={longEnough} active={password.length > 0}>
            At least {MIN_PASSWORD} characters
          </Rule>
        </ul>
      </div>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${idPrefix}-confirm`}>Confirm password</Label>
        <Input
          id={`${idPrefix}-confirm`}
          type="password"
          autoComplete="new-password"
          value={confirm}
          onChange={(e) => onConfirm(e.target.value)}
          aria-invalid={mismatch ? true : undefined}
          aria-describedby={`${idPrefix}-match`}
          className="h-control-md"
        />
        <p id={`${idPrefix}-match`} role={mismatch ? "alert" : undefined} className="min-h-4 text-xs">
          {mismatch ? (
            <span className="text-destructive">Passwords don't match</span>
          ) : matches ? (
            <span className="inline-flex items-center gap-1.5 text-success">
              <Check aria-hidden className="size-3" />
              Passwords match
            </span>
          ) : null}
        </p>
      </div>
    </>
  );
}

/** Create a superuser. Deliberately its own dialog rather than the generic
 * record drawer: `_superusers` has system fields the record editor would
 * happily show, and none of them should be touched by hand. */
function CreateSuperuserDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void;
}) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [role, setRole] = useState(ADMIN);
  const [seedOpen, setSeedOpen] = useState(open);

  if (seedOpen !== open) {
    setSeedOpen(open);
    if (open) {
      setEmail("");
      setPassword("");
      setConfirm("");
      setRole(ADMIN);
    }
  }

  const create = useMutation({
    mutationFn: () =>
      cb.collection(COLLECTION).create({ email: email.trim(), password, passwordConfirm: confirm, role }),
    onSuccess: () => {
      toast.success(`Created ${email.trim()}`);
      onCreated();
    },
    onError: (error) => {
      const { title, description } = failureDescription(error);
      toast.error(title, { description });
    },
  });

  const emailError = email.trim() && !EMAIL_RE.test(email.trim()) ? "Enter a valid email address" : null;
  const ready = Boolean(email.trim()) && !emailError && password.length >= MIN_PASSWORD && password === confirm;

  return (
    <Dialog open={open} onOpenChange={(next) => (!create.isPending || next) && onOpenChange(next)}>
      <DialogContent className="sm:max-w-md">
        <form
          noValidate
          className="grid gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            if (ready && !create.isPending) create.mutate();
          }}
        >
          <DialogHeader>
            <DialogTitle>New superuser</DialogTitle>
            <DialogDescription>
              They'll be able to read and write everything, including these settings. Share the password over a
              secure channel — they can change it after signing in.
            </DialogDescription>
          </DialogHeader>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="su-email">Email</Label>
            <Input
              id="su-email"
              type="email"
              autoComplete="off"
              autoFocus
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              aria-invalid={emailError ? true : undefined}
              aria-describedby={emailError ? "su-email-error" : undefined}
              className="h-control-md"
            />
            {emailError ? (
              <p id="su-email-error" role="alert" className="text-xs text-destructive">
                {emailError}
              </p>
            ) : null}
          </div>

          <PasswordFields
            idPrefix="su"
            password={password}
            confirm={confirm}
            onPassword={setPassword}
            onConfirm={setConfirm}
            passwordLabel="Password"
          />

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="su-role">Role</Label>
            <Select value={role} onValueChange={setRole}>
              <SelectTrigger id="su-role" className="h-control-md w-full" aria-describedby="su-role-help">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ADMIN}>Admin</SelectItem>
                <SelectItem value={OWNER}>Owner</SelectItem>
              </SelectContent>
            </Select>
            <p id="su-role-help" className="text-xs text-pretty text-muted-foreground">
              {role === OWNER
                ? "Owners have full access and can manage other superusers, including their roles."
                : "Admins have full access except managing other superusers' roles or accounts."}
            </p>
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={create.isPending}>
              Cancel
            </Button>
            <Button type="submit" disabled={!ready || create.isPending}>
              {create.isPending ? <Spinner className="size-3.5" /> : null}
              {create.isPending ? "Creating…" : "Create superuser"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** Set a new password on an existing superuser. */
function PasswordDialog({
  record,
  onOpenChange,
}: {
  record: RecordModel | null;
  onOpenChange: (open: boolean) => void;
}) {
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [seedId, setSeedId] = useState<string | null>(null);

  if (seedId !== (record?.id ?? null)) {
    setSeedId(record?.id ?? null);
    setPassword("");
    setConfirm("");
  }

  const update = useMutation({
    mutationFn: () => cb.collection(COLLECTION).update(record!.id, { password, passwordConfirm: confirm }),
    onSuccess: () => {
      toast.success("Password changed", {
        description: "Existing sessions for that account stay valid until they expire.",
      });
      onOpenChange(false);
    },
    onError: (error) => {
      const { title, description } = failureDescription(error);
      toast.error(title, { description });
    },
  });

  const ready = password.length >= MIN_PASSWORD && password === confirm;

  return (
    <Dialog open={record !== null} onOpenChange={(next) => (!update.isPending || next) && onOpenChange(next)}>
      <DialogContent className="sm:max-w-md">
        <form
          noValidate
          className="grid gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            if (ready && !update.isPending) update.mutate();
          }}
        >
          <DialogHeader>
            <DialogTitle>Change password</DialogTitle>
            <DialogDescription>
              For <span className="font-mono">{String(record?.email ?? "")}</span>. Existing sessions stay signed in
              until they expire — revoke them under Sessions to sign the account out everywhere.
            </DialogDescription>
          </DialogHeader>

          <PasswordFields
            idPrefix="su-new"
            password={password}
            confirm={confirm}
            onPassword={setPassword}
            onConfirm={setConfirm}
            passwordLabel="New password"
          />

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={update.isPending}>
              Cancel
            </Button>
            <Button type="submit" disabled={!ready || update.isPending}>
              {update.isPending ? <Spinner className="size-3.5" /> : null}
              {update.isPending ? "Saving…" : "Change password"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
