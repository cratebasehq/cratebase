import { useMemo, useState } from "react";
import { toast } from "sonner";
import { Plus, ShieldAlert, Trash2 } from "lucide-react";
import { describeFailure } from "@/lib/api";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { useCollections } from "@/hooks/use-collections";
import { SETTINGS_GROUPS } from "@/lib/settings-nav";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { TagInput } from "@/components/ui/tag-input";
import { StatusBadge } from "@/components/layout/stat-card";
import { SettingRow, SettingsPage, SettingsSaveBar, SettingsSection, ToggleSetting } from "@/components/settings/settings-form";
import { useSaveShortcut } from "@/components/settings/use-save-shortcut";

const ITEM = SETTINGS_GROUPS.flatMap((g) => g.tabs).find((t) => t.legacyPath === "/settings/network")!;

type RateLimitRule = ServerSettings["rateLimits"]["rules"][number];

/** The slice of settings this page owns — everything that decides *who* may
 * reach the API and *how fast*, as opposed to what the API does once they're
 * in. Kept separate from `ApplicationPage` for the same reason that page
 * keeps `batch`/`logs` apart from `smtp`/`s3`: independent save, independent
 * blast radius. */
type Draft = Pick<ServerSettings, "rateLimits" | "trustedProxy" | "superuserIPs">;

function draftOf(settings: ServerSettings): Draft {
  return {
    rateLimits: { ...settings.rateLimits, rules: settings.rateLimits.rules.map((r) => ({ ...r })) },
    trustedProxy: { ...settings.trustedProxy },
    superuserIPs: [...settings.superuserIPs],
  };
}

function emptyRule(label = ""): RateLimitRule {
  return { label, audience: "", duration: 10, maxRequests: 100 };
}

/** One-click starting points for a project with no rules yet. */
const SUGGESTED_RULES: { label: string; hint: string; rule: RateLimitRule }[] = [
  {
    label: "Throttle sign-in",
    hint: "*:auth — 5 per 3 s",
    rule: { label: "*:auth", audience: "", duration: 3, maxRequests: 5 },
  },
  {
    label: "Throttle creates",
    hint: "*:create — 20 per 5 s",
    rule: { label: "*:create", audience: "", duration: 5, maxRequests: 20 },
  },
  {
    label: "Cap the whole API",
    hint: "/api/ — 300 per 10 s",
    rule: { label: "/api/", audience: "", duration: 10, maxRequests: 300 },
  },
];

/** A literal IPv4/IPv6 address or CIDR range. Deliberately loose — the
 * point is to catch a typo before it is saved into an allow-list that can
 * lock a superuser out, not to be a parser. */
const IP_LIKE = /^[0-9a-fA-F:.]+(\/\d{1,3})?$/;

function badIps(values: string[]): string[] {
  return values.filter((v) => !IP_LIKE.test(v.trim()));
}

/** Mirrors `crates/server/src/routes/settings.rs`'s `validate()`: a rule
 * needs a label, and `maxRequests`/`duration` can't be negative. The server
 * is the source of truth — this only means the 400 comes back with an error
 * already visible instead of a generic toast. */
function ruleError(rule: RateLimitRule): string | null {
  if (!rule.label.trim()) return "Needs a label — a path prefix or a tag like *:auth";
  if (rule.duration < 1) return "The window must be at least 1 second";
  if (rule.maxRequests < 0) return "Max requests can't be negative";
  return null;
}

function validate(draft: Draft): string[] {
  const errors: string[] = [];
  draft.rateLimits.rules.forEach((rule, i) => {
    const error = ruleError(rule);
    if (error) errors.push(`Rule ${i + 1}: ${error}`);
  });
  const excluded = badIps(draft.rateLimits.excludedIPs);
  if (excluded.length > 0) errors.push(`Excluded IPs: "${excluded[0]}" isn't an IP address or CIDR range`);
  const superuser = badIps(draft.superuserIPs);
  if (superuser.length > 0) errors.push(`Superuser IPs: "${superuser[0]}" isn't an IP address or CIDR range`);
  return errors;
}

/** `""` matches every caller, `@guest`/`@auth` narrow to
 * unauthenticated or authenticated requests. There's no fourth option on
 * the wire. */
const AUDIENCES = [
  { value: "", label: "Anyone" },
  { value: "@guest", label: "Guests only" },
  { value: "@auth", label: "Authenticated only" },
];

/** Radix Select can't hold an empty-string value, so "Anyone" is shown under this key. */
const ANY = "__any";

/** The record-endpoint verbs `tags_for` (`crates/server/src/middleware/rate_limit.rs`)
 * derives from the URL shape. A rule labelled `{collection}:{action}` only
 * ever matches that one collection's records endpoint for that one verb —
 * this is what makes per-collection, per-verb limiting possible without a
 * dedicated schema field. */
const RECORD_ACTIONS = ["list", "view", "create", "update", "delete"] as const;

/** Real collection names turn "type the tag by hand and hope you spelled
 * the collection right" into "pick it off a list". The label field stays a
 * free-text input — tags like `*:auth` and path rules like `/api/batch`
 * have no collection to suggest — this only adds suggestions on top. */
function useRuleLabelSuggestions(): string[] {
  const { data: collections } = useCollections();
  return useMemo(() => {
    if (!collections) return [];
    const suggestions: string[] = [];
    for (const collection of collections) {
      for (const action of RECORD_ACTIONS) {
        suggestions.push(`${collection.name}:${action}`);
      }
    }
    return suggestions;
  }, [collections]);
}

/** "At most 100 requests per 10 s, for anyone" — the rule read back as a sentence. */
function ruleSummary(rule: RateLimitRule): string {
  const who = { "": "anyone", "@guest": "guests", "@auth": "signed-in users" }[rule.audience] ?? "anyone";
  const unit = rule.duration === 1 ? "second" : `${rule.duration} seconds`;
  return `At most ${rule.maxRequests.toLocaleString()} request${rule.maxRequests === 1 ? "" : "s"} per ${unit}, for ${who}.`;
}

const RULE_GRID = "sm:grid-cols-[minmax(0,1fr)_10rem_6rem_6rem_auto]";

function NetworkSkeleton() {
  return (
    <SettingsPage title={ITEM.label} description={ITEM.description} width="form">
      {Array.from({ length: 3 }).map((_, i) => (
        <Skeleton key={i} className="h-32 w-full" />
      ))}
    </SettingsPage>
  );
}

export function NetworkPage() {
  const { data: settings, isPending } = useSettings();
  const save = useSettingsMutation();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [seedKey, setSeedKey] = useState<ServerSettings | undefined>(settings);
  const labelSuggestions = useRuleLabelSuggestions();

  if (settings && (seedKey !== settings || draft === null)) {
    setSeedKey(settings);
    setDraft(draftOf(settings));
  }

  const errors = useMemo(() => (draft ? validate(draft) : []), [draft]);
  const dirty = draft && settings ? JSON.stringify(draft) !== JSON.stringify(draftOf(settings)) : false;

  function submit() {
    if (!draft || errors.length > 0) return;
    save.mutate(
      { rateLimits: draft.rateLimits, trustedProxy: draft.trustedProxy, superuserIPs: draft.superuserIPs },
      {
        onSuccess: () => toast.success("Network settings saved"),
        onError: (error) => {
          const failure = describeFailure(error);
          const fields = Object.entries(failure.fields).map(([k, v]) => `${k}: ${v}`);
          toast.error(failure.title, {
            description: fields.length > 0 ? fields.join("; ") : failure.serverMessage || failure.detail,
          });
        },
      },
    );
  }
  useSaveShortcut({ dirty, canSave: errors.length === 0 && !save.isPending, onSave: submit });

  if (isPending || !draft || !settings) return <NetworkSkeleton />;

  function patch(next: Partial<Draft>) {
    setDraft((d) => (d ? { ...d, ...next } : d));
  }

  function patchRule(index: number, next: Partial<RateLimitRule>) {
    if (!draft) return;
    const rules = draft.rateLimits.rules.map((rule, i) => (i === index ? { ...rule, ...next } : rule));
    patch({ rateLimits: { ...draft.rateLimits, rules } });
  }

  function addRule(rule: RateLimitRule = emptyRule()) {
    if (!draft) return;
    patch({ rateLimits: { ...draft.rateLimits, rules: [...draft.rateLimits.rules, rule] } });
  }

  function removeRule(index: number) {
    if (!draft) return;
    patch({ rateLimits: { ...draft.rateLimits, rules: draft.rateLimits.rules.filter((_, i) => i !== index) } });
  }

  const rules = draft.rateLimits.rules;
  const superuserIPsChanged = JSON.stringify(draft.superuserIPs) !== JSON.stringify(settings.superuserIPs);

  return (
    <SettingsPage title={ITEM.label} description={ITEM.description} width="form">
      <SettingsSection
        title="Rate limiting"
        description="Cap how often a client can call the API. Rules are checked in order; the first one whose label matches the request wins."
        action={
          <Button type="button" variant="outline" size="sm" onClick={() => addRule()}>
            <Plus />
            Add rule
          </Button>
        }
      >
        <SettingRow label="Enabled" htmlFor="rl-enabled" help="Off keeps your rules but stops enforcing them.">
          <div className="flex items-center gap-2">
            <ToggleSetting
              id="rl-enabled"
              checked={draft.rateLimits.enabled}
              onChange={(enabled) => patch({ rateLimits: { ...draft.rateLimits, enabled } })}
              label={draft.rateLimits.enabled ? "Enforcing the rules below" : "Not enforced"}
            />
            {draft.rateLimits.enabled && rules.length === 0 ? (
              <StatusBadge tone="warning">No rules yet</StatusBadge>
            ) : null}
          </div>
        </SettingRow>

        <div className={cn("flex flex-col gap-2", !draft.rateLimits.enabled && rules.length > 0 && "opacity-60")}>
          {rules.length === 0 ? (
            <div className="flex flex-col gap-2 rounded-lg border border-dashed border-border px-3 py-4">
              <p className="text-sm text-muted-foreground">
                No rules — every request is unlimited. Start from a suggestion, or add your own.
              </p>
              <div className="flex flex-wrap gap-2">
                {SUGGESTED_RULES.map((suggestion) => (
                  <Button
                    key={suggestion.label}
                    type="button"
                    variant="outline"
                    size="sm"
                    title={suggestion.hint}
                    onClick={() => addRule({ ...suggestion.rule })}
                  >
                    <Plus />
                    {suggestion.label}
                  </Button>
                ))}
              </div>
            </div>
          ) : (
            <>
              <div
                aria-hidden
                className={cn("hidden gap-2 px-2.5 text-xs text-muted-foreground sm:grid", RULE_GRID)}
              >
                <span>Label</span>
                <span>Applies to</span>
                <span>Window (s)</span>
                <span>Max requests</span>
                <span className="w-control-sm" />
              </div>
              {rules.map((rule, i) => {
                const error = ruleError(rule);
                return (
                  <div key={i} className="rounded-lg border border-border bg-background/50 p-2.5">
                    <div className={cn("grid grid-cols-1 items-center gap-2", RULE_GRID)}>
                      <Input
                        value={rule.label}
                        onChange={(e) => patchRule(i, { label: e.target.value })}
                        placeholder="/api/, *:auth, posts:list…"
                        aria-label={`Rule ${i + 1} label`}
                        aria-invalid={!rule.label.trim() || undefined}
                        list="rate-limit-rule-label-suggestions"
                        className="h-control-sm font-mono text-xs"
                      />
                      <Select
                        value={rule.audience || ANY}
                        onValueChange={(audience) => patchRule(i, { audience: audience === ANY ? "" : audience })}
                      >
                        <SelectTrigger size="sm" aria-label={`Rule ${i + 1} audience`} className="h-control-sm w-full text-xs">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {AUDIENCES.map((a) => (
                            <SelectItem key={a.value} value={a.value || ANY}>
                              {a.label}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      <Input
                        type="number"
                        min={1}
                        value={rule.duration}
                        onChange={(e) => patchRule(i, { duration: e.target.value === "" ? 1 : Number(e.target.value) })}
                        aria-label={`Rule ${i + 1} window, seconds`}
                        aria-invalid={rule.duration < 1 || undefined}
                        title="Window, in seconds"
                        className="h-control-sm font-tabular text-xs"
                      />
                      <Input
                        type="number"
                        min={0}
                        value={rule.maxRequests}
                        onChange={(e) =>
                          patchRule(i, { maxRequests: e.target.value === "" ? 0 : Number(e.target.value) })
                        }
                        aria-label={`Rule ${i + 1} max requests`}
                        aria-invalid={rule.maxRequests < 0 || undefined}
                        title="Max requests per window"
                        className="h-control-sm font-tabular text-xs"
                      />
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Remove rule ${i + 1}`}
                        onClick={() => removeRule(i)}
                      >
                        <Trash2 className="text-destructive" />
                      </Button>
                    </div>
                    <p
                      role={error ? "alert" : undefined}
                      className={cn("mt-1.5 text-xs", error ? "text-destructive" : "text-muted-foreground")}
                    >
                      {error ?? ruleSummary(rule)}
                    </p>
                  </div>
                );
              })}
            </>
          )}
          {/* Native datalist, not a `Command`/`Popover` combobox: the label
           * stays free text (tags, prefixes and exact paths all live in the
           * same field), this only autocompletes real `collection:action`
           * pairs so a rule can be scoped to one collection without typing
           * its name — and possibly misspelling it — by hand. */}
          <datalist id="rate-limit-rule-label-suggestions">
            {labelSuggestions.map((s) => (
              <option key={s} value={s} />
            ))}
          </datalist>
          <p className="text-xs leading-snug text-pretty text-muted-foreground">
            Label is a path prefix (<code className="font-mono">/api/</code>), an exact path, or a tag (
            <code className="font-mono">*:auth</code>, <code className="font-mono">*:create</code>,{" "}
            <code className="font-mono">posts:list</code>) — start typing a collection name for suggestions scoped to
            that collection's list/view/create/update/delete endpoints.
          </p>
        </div>

        <TagInput
          label="Excluded IPs"
          value={draft.rateLimits.excludedIPs}
          onChange={(excludedIPs) => patch({ rateLimits: { ...draft.rateLimits, excludedIPs } })}
          placeholder="e.g. 127.0.0.1"
          hint="These addresses skip rate limiting entirely — useful for internal health checks."
          validate={(candidate) => IP_LIKE.test(candidate.trim())}
        />
      </SettingsSection>

      <SettingsSection
        title="Trusted proxy"
        description="How the real client IP is recovered when Cratebase sits behind a reverse proxy or load balancer."
      >
        <TagInput
          label="Trusted headers"
          value={draft.trustedProxy.headers}
          onChange={(headers) => patch({ trustedProxy: { ...draft.trustedProxy, headers } })}
          placeholder="e.g. X-Forwarded-For"
          hint="Checked in order; the first present header wins. Empty means the direct socket address is used."
        />
        <SettingRow
          label="Leftmost IP"
          htmlFor="rl-leftmost"
          help="X-Forwarded-For is a comma-separated chain added to by every hop. Leftmost is the original client; rightmost is the proxy closest to this server. Only enable this if every hop in front of Cratebase is trusted, or a client can spoof its own IP."
        >
          <ToggleSetting
            id="rl-leftmost"
            checked={draft.trustedProxy.useLeftmostIP}
            onChange={(useLeftmostIP) => patch({ trustedProxy: { ...draft.trustedProxy, useLeftmostIP } })}
            label={draft.trustedProxy.useLeftmostIP ? "Using the leftmost address" : "Using the rightmost address"}
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection
        title="Superuser IPs"
        description="Restrict which addresses may authenticate as a superuser, on top of whatever credentials they present."
      >
        <TagInput
          label="Allowed IPs"
          value={draft.superuserIPs}
          onChange={(superuserIPs) => patch({ superuserIPs })}
          placeholder="e.g. 10.0.0.0/8"
          hint="Leave empty to allow superuser sign-in from any address."
          validate={(candidate) => IP_LIKE.test(candidate.trim())}
        />
        {superuserIPsChanged && draft.superuserIPs.length > 0 ? (
          <p
            role="note"
            className="flex items-start gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning"
          >
            <ShieldAlert className="mt-0.5 size-3.5 shrink-0" />
            <span>
              Once saved, only these addresses can sign in as a superuser. Make sure the one you're using now is on the
              list, or you will lock yourself out of this dashboard.
            </span>
          </p>
        ) : null}
      </SettingsSection>

      <SettingsSaveBar
        dirty={dirty}
        pending={save.isPending}
        errors={errors}
        onSave={submit}
        onReset={() => setDraft(draftOf(settings))}
      />
    </SettingsPage>
  );
}
