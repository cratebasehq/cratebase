import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Check, ChevronDown, ListChecks, X } from "lucide-react";
import type { CollectionModel } from "@cratebase/client";
import { cb } from "@/lib/api";
import { cn } from "@/lib/utils";
import { useSettings } from "@/hooks/use-settings";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";

const DISMISSED_KEY = "cratebase.onboarding.dismissed";
const MANUAL_DONE_KEY = "cratebase.onboarding.manualDone";

function readDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISSED_KEY) === "1";
  } catch {
    return false;
  }
}

function writeDismissed(value: boolean) {
  try {
    if (value) localStorage.setItem(DISMISSED_KEY, "1");
    else localStorage.removeItem(DISMISSED_KEY);
  } catch {
    // Private browsing / blocked storage — the checklist just reappears
    // next load, which is a fine fallback for a dismiss button.
  }
}

/** A handful of items (e.g. "email templates reviewed") have no server
 * state to compute "done" from — they're marked done by hand, and that
 * mark is remembered the same way dismissal is. */
function readManualDone(): Set<string> {
  try {
    const raw = localStorage.getItem(MANUAL_DONE_KEY);
    return raw ? new Set(JSON.parse(raw) as string[]) : new Set();
  } catch {
    return new Set();
  }
}

function writeManualDone(ids: Set<string>) {
  try {
    localStorage.setItem(MANUAL_DONE_KEY, JSON.stringify([...ids]));
  } catch {
    // Same fallback as `writeDismissed` — worst case it asks again.
  }
}

interface ChecklistItem {
  id: string;
  label: string;
  done: boolean;
  optional?: boolean;
  /** No server state to compute `done` from — clicking the row's check
   * itself toggles it, instead of navigating. */
  manual?: boolean;
  to: string;
  search?: Record<string, string>;
  params?: Record<string, string>;
}

/** A collection's own sign-in methods — any one of these being on counts
 * as "configured" for the checklist item, mirroring what
 * `AuthOptionsEditor` lets a collection turn on. */
function hasSignInMethod(collection: CollectionModel): boolean {
  return Boolean(
    collection.passwordAuth?.enabled ||
      collection.otp?.enabled ||
      collection.magicLink?.enabled ||
      collection.oauth2?.enabled,
  );
}

/**
 * A dismissible checklist of the handful of things a fresh instance
 * usually still needs — computed from the same settings/collections APIs
 * every settings page already reads, not a separate tracked "onboarding"
 * concept server-side. Dismissal is per-browser (`localStorage`); there's
 * nothing here worth syncing across devices or superusers.
 */
export function OnboardingChecklist({ collections }: { collections: CollectionModel[] | undefined }) {
  const [dismissed, setDismissed] = useState(() => readDismissed());
  const [manualDone, setManualDone] = useState(() => readManualDone());
  const [open, setOpen] = useState(true);

  const { data: settings } = useSettings();

  // Cheap existence checks — `perPage: 1` with `skipTotal` off just to
  // read `totalItems`, same pattern the home screen's own record counts
  // already use.
  const mailSent = useQuery({
    queryKey: ["onboarding", "mail-log-any"],
    queryFn: () => cb.collection("_mailLog").list({ page: 1, perPage: 1, filter: 'status = "sent"' }),
    enabled: Boolean(settings?.smtp.enabled),
    staleTime: 60_000,
  });

  if (dismissed || !settings) return null;

  const authCollections = (collections ?? []).filter((c) => c.type === "auth");

  const items: ChecklistItem[] = [
    {
      id: "create-superuser",
      label: "Create a superuser account",
      // Trivially true — reaching this screen at all requires being
      // signed in as one already.
      done: true,
      to: "/settings/auth",
      search: { tab: "superusers" },
    },
    {
      id: "app-identity",
      label: "Set your application name and URL",
      done: settings.meta.appName.trim() !== "" && settings.meta.appName !== "Acme" && settings.meta.appURL.trim() !== "",
      to: "/settings/application",
      search: { tab: "general" },
    },
    {
      id: "branding",
      label: "Add a logo or brand color",
      done: Boolean(settings.meta.logoUrl.trim() || settings.meta.brandColor.trim()),
      to: "/settings/application",
      search: { tab: "branding" },
    },
    {
      id: "mail-delivery",
      label: "Configure real mail delivery and send a test email",
      done: settings.smtp.enabled && (mailSent.data?.totalItems ?? 0) > 0,
      to: "/settings/email",
      search: { tab: "delivery" },
    },
    {
      id: "auth-method",
      label: "Set up a sign-in method on an auth collection",
      done: authCollections.some(hasSignInMethod),
      to: authCollections[0] ? "/collections/$name" : "/",
      params: authCollections[0] ? { name: authCollections[0].name } : undefined,
    },
    {
      id: "email-templates-reviewed",
      label: "Review the email templates",
      done: manualDone.has("email-templates-reviewed"),
      manual: true,
      to: "/settings/email",
      search: { tab: "templates" },
    },
    {
      id: "backups",
      label: "Schedule automatic backups",
      done: Boolean(settings.backups.cron.trim()),
      to: "/settings/database",
      search: { tab: "backups" },
    },
    {
      id: "rate-limiting",
      label: "Turn on rate limiting",
      done: settings.rateLimits.enabled,
      to: "/settings/auth",
      search: { tab: "network" },
    },
    {
      id: "s3-storage",
      label: "Connect S3-compatible file storage",
      done: settings.s3.enabled,
      optional: true,
      to: "/settings/email",
      search: { tab: "delivery" },
    },
  ];

  const remaining = items.filter((i) => !i.done && !i.optional).length;

  return (
    <Collapsible
      open={open}
      onOpenChange={setOpen}
      className="flex flex-col gap-2 rounded-lg border border-border bg-card p-3"
    >
      <div className="flex items-center justify-between gap-2">
        <CollapsibleTrigger asChild>
          <button type="button" className="flex min-w-0 items-center gap-2 text-left">
            <ListChecks className="size-4 shrink-0 text-muted-foreground" />
            <span className="text-sm font-medium">
              Getting started {remaining > 0 ? `— ${remaining} left` : "— all set"}
            </span>
            <ChevronDown className={cn("size-3.5 shrink-0 text-muted-foreground transition-transform", open && "rotate-180")} />
          </button>
        </CollapsibleTrigger>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label="Dismiss checklist"
          className="shrink-0 text-muted-foreground"
          onClick={() => {
            writeDismissed(true);
            setDismissed(true);
          }}
        >
          <X className="size-3.5" />
        </Button>
      </div>
      <CollapsibleContent>
        <div className="grid grid-cols-1 gap-1.5 pt-1 sm:grid-cols-2">
          {items.map((item) => {
            const check = (
              <span
                role={item.manual ? "button" : undefined}
                aria-label={item.manual ? (item.done ? "Mark as not done" : "Mark as done") : undefined}
                onClick={
                  item.manual
                    ? (e) => {
                        e.preventDefault();
                        setManualDone((prev) => {
                          const next = new Set(prev);
                          if (next.has(item.id)) next.delete(item.id);
                          else next.add(item.id);
                          writeManualDone(next);
                          return next;
                        });
                      }
                    : undefined
                }
                className={cn(
                  "grid size-4 shrink-0 place-items-center rounded-full border",
                  item.done ? "border-transparent bg-primary text-primary-foreground" : "border-border text-transparent",
                  item.manual && "cursor-pointer",
                )}
              >
                <Check className="size-2.5" />
              </span>
            );
            return (
              <Link
                key={item.id}
                to={item.to}
                search={item.search as never}
                params={item.params as never}
                className="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors hover:bg-accent"
              >
                {check}
                <span className={cn("min-w-0 flex-1 truncate", item.done && "text-muted-foreground line-through")}>
                  {item.label}
                </span>
                {item.optional ? <span className="shrink-0 text-2xs text-muted-foreground">optional</span> : null}
              </Link>
            );
          })}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
