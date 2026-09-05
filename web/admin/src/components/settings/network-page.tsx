import { useMemo, useState } from "react";
import { toast } from "sonner";
import { Plus, Trash2 } from "lucide-react";
import { describeFailure } from "@/lib/api";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { useCollections } from "@/hooks/use-collections";
import { settingsItemFor } from "@/lib/settings-nav";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { TagInput } from "@/components/ui/tag-input";
import { SettingRow, SettingsPage, SettingsSaveBar, SettingsSection, ToggleSetting } from "@/components/settings/settings-form";

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

function emptyRule(): RateLimitRule {
  return { label: "", audience: "", duration: 10, maxRequests: 100 };
}

/** Mirrors `crates/server/src/routes/settings.rs`'s `validate()`: a rule
 * needs a label, and `maxRequests`/`duration` can't be negative. The server
 * is the source of truth — this only means the 400 comes back with an error
 * already visible instead of a generic toast. */
function validate(draft: Draft): string[] {
  const errors: string[] = [];
  draft.rateLimits.rules.forEach((rule, i) => {
    if (!rule.label.trim()) errors.push(`Rule ${i + 1} needs a label`);
    if (rule.maxRequests < 0) errors.push(`Rule ${i + 1}'s max requests can't be negative`);
    if (rule.duration < 1) errors.push(`Rule ${i + 1}'s window must be at least 1 second`);
  });
  return errors;
}

/** `""` matches every caller, PocketBase's own `@guest`/`@auth` narrow to
 * unauthenticated or authenticated requests. There's no fourth option on
 * the wire. */
const AUDIENCES = [
  { value: "", label: "Anyone" },
  { value: "@guest", label: "Guests only" },
  { value: "@auth", label: "Authenticated only" },
];

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

  if (isPending || !draft || !settings) {
    return (
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-page">
        {Array.from({ length: 3 }).map((_, i) => (
          <Skeleton key={i} className="h-32 w-full" />
        ))}
      </div>
    );
  }

  const dirty = JSON.stringify(draft) !== JSON.stringify(draftOf(settings));

  function patch(next: Partial<Draft>) {
    setDraft((d) => (d ? { ...d, ...next } : d));
  }

  function patchRule(index: number, next: Partial<RateLimitRule>) {
    if (!draft) return;
    const rules = draft.rateLimits.rules.map((rule, i) => (i === index ? { ...rule, ...next } : rule));
    patch({ rateLimits: { ...draft.rateLimits, rules } });
  }

  function addRule() {
    if (!draft) return;
    patch({ rateLimits: { ...draft.rateLimits, rules: [...draft.rateLimits.rules, emptyRule()] } });
  }

  function removeRule(index: number) {
    if (!draft) return;
    patch({ rateLimits: { ...draft.rateLimits, rules: draft.rateLimits.rules.filter((_, i) => i !== index) } });
  }

  function submit() {
    if (!draft || errors.length > 0) return;
    save.mutate(
      { rateLimits: draft.rateLimits, trustedProxy: draft.trustedProxy, superuserIPs: draft.superuserIPs },
      {
        onSuccess: () => toast.success("Settings saved"),
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

  const item = settingsItemFor("/settings/network")!;
  return (
    <SettingsPage title={item.label} description={item.description} width="form">
      <SettingsSection
        title="Rate limiting"
        description="Cap how often a client can call the API. Rules are checked in order; the first one whose label matches the request wins."
        action={
          <Button type="button" variant="outline" size="sm" className="h-control-sm gap-1.5" onClick={addRule}>
            <Plus className="size-3.5" />
            Add rule
          </Button>
        }
      >
        <SettingRow label="Enabled" htmlFor="rl-enabled">
          <ToggleSetting
            id="rl-enabled"
            checked={draft.rateLimits.enabled}
            onChange={(enabled) => patch({ rateLimits: { ...draft.rateLimits, enabled } })}
            label={draft.rateLimits.enabled ? "Enforcing the rules below" : "Not enforced"}
          />
        </SettingRow>

        <div className="flex flex-col gap-2">
          {draft.rateLimits.rules.length === 0 ? (
            <p className="text-xs text-muted-foreground">No rules — every request is unlimited.</p>
          ) : (
            draft.rateLimits.rules.map((rule, i) => (
              <div
                key={i}
                className="grid grid-cols-1 items-center gap-2 rounded-lg border border-border p-2.5 sm:grid-cols-[1fr_9rem_5.5rem_5.5rem_auto]"
              >
                <Input
                  value={rule.label}
                  onChange={(e) => patchRule(i, { label: e.target.value })}
                  placeholder="/api/, *:auth, posts:list…"
                  aria-label={`Rule ${i + 1} label`}
                  list="rate-limit-rule-label-suggestions"
                  className="h-control-sm font-mono text-xs"
                />
                <Select value={rule.audience} onValueChange={(audience) => patchRule(i, { audience })}>
                  <SelectTrigger aria-label={`Rule ${i + 1} audience`} className="h-control-sm text-xs">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {AUDIENCES.map((a) => (
                      <SelectItem key={a.value} value={a.value}>
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
                  <Trash2 className="size-3.5 text-destructive" />
                </Button>
              </div>
            ))
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
          <p className="text-2xs leading-snug text-muted-foreground">
            Label is a path prefix (<code className="font-mono">/api/</code>), an exact path, or a tag (
            <code className="font-mono">*:auth</code>, <code className="font-mono">*:create</code>,{" "}
            <code className="font-mono">posts:list</code>) — start typing a collection name for suggestions
            scoped to that collection's list/view/create/update/delete endpoints. Window and max requests
            define "at most N requests per window seconds".
          </p>
        </div>

        <TagInput
          label="Excluded IPs"
          value={draft.rateLimits.excludedIPs}
          onChange={(excludedIPs) => patch({ rateLimits: { ...draft.rateLimits, excludedIPs } })}
          placeholder="e.g. 127.0.0.1"
          hint="These addresses skip rate limiting entirely — useful for internal health checks."
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
        />
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
