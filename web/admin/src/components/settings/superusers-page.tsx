import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { KeyRound, Plus, ShieldUser, Trash2 } from "lucide-react";
import type { RecordModel } from "pocketbase";
import { cb, currentSuperuser, describeFailure } from "@/lib/api";
import { settingsItemFor } from "@/lib/settings-nav";
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
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { SettingsPage } from "@/components/settings/settings-form";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";

const COLLECTION = "_superusers";
const MIN_PASSWORD = 8;
/** `_superusers.role` values (`cratebase_core::SUPERUSER_ROLE_OWNER`/
 * `SUPERUSER_ROLE_ADMIN` on the server). Owners can manage every other
 * superuser account; admins have full dashboard/API access otherwise. */
const OWNER = "owner";
const ADMIN = "admin";

/**
 * Superusers are ordinary auth records in `_superusers` (PocketBase v0.23+
 * dropped `/api/admins`), so this is a small CRUD over that collection —
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
    queryFn: () => cb.collection(COLLECTION).getFullList<RecordModel>({ sort: "created" }),
  });

  function invalidate() {
    return queryClient.invalidateQueries({ queryKey: ["superusers"] });
  }

  const remove = useMutation({
    mutationFn: (id: string) => cb.collection(COLLECTION).delete(id),
    onSuccess: async () => {
      await invalidate();
      toast.success("Superuser removed");
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.serverMessage || failure.detail });
    },
  });

  const changeRole = useMutation({
    mutationFn: ({ id, role }: { id: string; role: string }) => cb.collection(COLLECTION).update(id, { role }),
    onSuccess: async () => {
      await invalidate();
      toast.success("Role updated");
    },
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error(failure.title, { description: failure.serverMessage || failure.detail });
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
      width="form"
      action={
        isOwner ? (
          <Button size="sm" className="gap-1.5" onClick={() => setCreating(true)}>
            <Plus className="size-3.5" />
            New superuser
          </Button>
        ) : undefined
      }
    >

      {superusers.isPending ? (
        <div className="flex flex-col gap-1">
          {Array.from({ length: 2 }).map((_, i) => (
            <Skeleton key={i} className="h-control-lg w-full" />
          ))}
        </div>
      ) : superusers.isError ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon" className="text-destructive">
              <ShieldUser />
            </EmptyMedia>
            <EmptyTitle>Couldn't load superusers</EmptyTitle>
            <EmptyDescription>{describeFailure(superusers.error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <div className="overflow-hidden rounded-lg border border-border">
          {rows.map((row) => {
            const isMe = row.id === me?.id;
            const role = typeof row.role === "string" && row.role ? row.role : ADMIN;
            // The lockout the server refuses to let happen: the last
            // owner can't be re-roled or removed, by anyone (in
            // practice always the viewer's own row — nobody else could
            // be the sole owner while also being able to act here).
            const isSoleOwner = role === OWNER && ownerCount <= 1;
            return (
              <div
                key={row.id}
                className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2 last:border-b-0"
              >
                <ShieldUser className="size-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate text-sm">{String(row.email ?? row.id)}</span>
                {isMe ? (
                  <Badge variant="outline" className="shrink-0 font-normal">
                    you
                  </Badge>
                ) : null}
                {isOwner ? (
                  <Select
                    value={role}
                    disabled={isSoleOwner || changeRole.isPending}
                    onValueChange={(next) => changeRole.mutate({ id: row.id, role: next })}
                  >
                    <SelectTrigger
                      size="sm"
                      className="h-control-sm w-28 shrink-0"
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
                  <Badge variant="outline" className="shrink-0 font-normal capitalize">
                    {role}
                  </Badge>
                )}
                <span className="shrink-0 font-mono text-2xs text-muted-foreground">
                  {row.created ? new Date(row.created).toLocaleDateString() : ""}
                </span>
                {isOwner || isMe ? (
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-control-sm shrink-0 gap-1.5"
                    onClick={() => setResetting(row)}
                  >
                    <KeyRound className="size-3.5" />
                    Password
                  </Button>
                ) : null}
                {isOwner ? (
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Remove ${String(row.email ?? row.id)}`}
                    className="shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                    disabled={isSoleOwner}
                    title={isSoleOwner ? "The last remaining owner can't be removed" : undefined}
                    onClick={() => setDeleting(row)}
                  >
                    <Trash2 className="size-3.5" />
                  </Button>
                ) : null}
              </div>
            );
          })}
        </div>
      )}

      <p className="text-2xs leading-snug text-muted-foreground">
        A superuser can also be created from the command line with{" "}
        <code className="font-mono">cratebase superuser create &lt;email&gt; &lt;password&gt;</code> — the way back in
        if you lock yourself out. Command-line superusers are always created as owners.
      </p>

      <SuperuserSheet
        open={creating}
        onOpenChange={setCreating}
        onCreated={() => {
          void invalidate();
          setCreating(false);
        }}
      />

      <PasswordSheet record={resetting} onOpenChange={(open) => !open && setResetting(null)} />

      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove this superuser?</AlertDialogTitle>
            <AlertDialogDescription>
              <span className="font-mono">{String(deleting?.email ?? "")}</span> loses access to this dashboard and
              the whole API immediately.
              {deleting?.id === me?.id ? " This is the account you are signed in with — you will be logged out." : ""}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                const target = deleting;
                setDeleting(null);
                if (target) remove.mutate(target.id);
              }}
            >
              Remove superuser
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </SettingsPage>
  );
}

/** Create a superuser. Deliberately its own sheet rather than the generic
 * record drawer: `_superusers` has system fields the record editor would
 * happily show, and none of them should be touched by hand. */
function SuperuserSheet({
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
  const [role, setRole] = useState(ADMIN);
  const [seedOpen, setSeedOpen] = useState(open);

  if (seedOpen !== open) {
    setSeedOpen(open);
    if (open) {
      setEmail("");
      setPassword("");
      setRole(ADMIN);
    }
  }

  const create = useMutation({
    mutationFn: () =>
      cb.collection(COLLECTION).create({ email, password, passwordConfirm: password, role }),
    onSuccess: () => {
      toast.success("Superuser created");
      onCreated();
    },
    onError: (error) => {
      const failure = describeFailure(error);
      const fields = Object.entries(failure.fields).map(([k, v]) => `${k}: ${v}`);
      toast.error(failure.title, {
        description: fields.length > 0 ? fields.join("; ") : failure.serverMessage || failure.detail,
      });
    },
  });

  const emailError = email.trim() && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email) ? "Not a valid email address" : null;
  const passwordError = password && password.length < MIN_PASSWORD ? `At least ${MIN_PASSWORD} characters` : null;
  const ready = Boolean(email.trim()) && password.length >= MIN_PASSWORD && !emailError;

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="gap-0 p-0 data-[side=right]:w-full data-[side=right]:sm:max-w-[440px]">
        <form
          noValidate
          onSubmit={(e) => {
            e.preventDefault();
            if (ready && !create.isPending) create.mutate();
          }}
          className="flex h-full min-h-0 flex-col"
        >
          <SheetHeader>
            <SheetTitle>New superuser</SheetTitle>
            <SheetDescription>
              They will be able to read and write everything, including these settings.
            </SheetDescription>
          </SheetHeader>

          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-4 pb-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="su-email">Email</Label>
              <Input
                id="su-email"
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                aria-invalid={emailError ? true : undefined}
                className="h-control-md"
              />
              {emailError ? <p className="text-xs text-destructive">{emailError}</p> : null}
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="su-password">Password</Label>
              <Input
                id="su-password"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={`At least ${MIN_PASSWORD} characters`}
                aria-invalid={passwordError ? true : undefined}
                className="h-control-md"
              />
              {passwordError ? <p className="text-xs text-destructive">{passwordError}</p> : null}
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="su-role">Role</Label>
              <Select value={role} onValueChange={setRole}>
                <SelectTrigger id="su-role" className="h-control-md">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={ADMIN}>Admin</SelectItem>
                  <SelectItem value={OWNER}>Owner</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-2xs text-muted-foreground">
                {role === OWNER
                  ? "Owners have full access and can manage other superusers, including their roles."
                  : "Admins have full access except managing other superusers' roles or accounts."}
              </p>
            </div>
          </div>

          <SheetFooter className="flex-row justify-end border-t border-border">
            <Button type="button" variant="ghost" size="sm" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={!ready || create.isPending}>
              {create.isPending ? <Spinner /> : null}
              {create.isPending ? "Creating…" : "Create superuser"}
            </Button>
          </SheetFooter>
        </form>
      </SheetContent>
    </Sheet>
  );
}

/** Set a new password on an existing superuser. */
function PasswordSheet({
  record,
  onOpenChange,
}: {
  record: RecordModel | null;
  onOpenChange: (open: boolean) => void;
}) {
  const [password, setPassword] = useState("");
  const [seedId, setSeedId] = useState<string | null>(null);

  if (seedId !== (record?.id ?? null)) {
    setSeedId(record?.id ?? null);
    setPassword("");
  }

  const update = useMutation({
    mutationFn: () =>
      cb.collection(COLLECTION).update(record!.id, { password, passwordConfirm: password }),
    onSuccess: () => {
      toast.success("Password changed", {
        description: "Existing sessions for that account stay valid until they expire.",
      });
      onOpenChange(false);
    },
    onError: (error) => {
      const failure = describeFailure(error);
      const fields = Object.entries(failure.fields).map(([k, v]) => `${k}: ${v}`);
      toast.error(failure.title, {
        description: fields.length > 0 ? fields.join("; ") : failure.serverMessage || failure.detail,
      });
    },
  });

  const tooShort = password.length > 0 && password.length < MIN_PASSWORD;

  return (
    <Sheet open={record !== null} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="gap-0 p-0 data-[side=right]:w-full data-[side=right]:sm:max-w-[440px]">
        <form
          noValidate
          onSubmit={(e) => {
            e.preventDefault();
            if (password.length >= MIN_PASSWORD && !update.isPending) update.mutate();
          }}
          className="flex h-full min-h-0 flex-col"
        >
          <SheetHeader>
            <SheetTitle>Change password</SheetTitle>
            <SheetDescription>
              For <span className="font-mono">{String(record?.email ?? "")}</span>.
            </SheetDescription>
          </SheetHeader>

          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-4 pb-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="su-newpass">New password</Label>
              <Input
                id="su-newpass"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={`At least ${MIN_PASSWORD} characters`}
                aria-invalid={tooShort ? true : undefined}
                className="h-control-md"
              />
              {tooShort ? <p className="text-xs text-destructive">At least {MIN_PASSWORD} characters</p> : null}
            </div>
          </div>

          <SheetFooter className="flex-row justify-end border-t border-border">
            <Button type="button" variant="ghost" size="sm" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={password.length < MIN_PASSWORD || update.isPending}>
              {update.isPending ? <Spinner /> : null}
              {update.isPending ? "Saving…" : "Change password"}
            </Button>
          </SheetFooter>
        </form>
      </SheetContent>
    </Sheet>
  );
}
