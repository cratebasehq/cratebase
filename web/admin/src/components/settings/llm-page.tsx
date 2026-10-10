import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import { Coins, Plus, Trash2 } from "lucide-react";
import { cb, describeFailure } from "@/lib/api";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { SETTINGS_GROUPS } from "@/lib/settings-nav";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { StatCard, StatusBadge } from "@/components/layout/stat-card";
import {
  SecretSetting,
  SettingRow,
  SettingsPage,
  SettingsSaveBar,
  SettingsSection,
  TextSetting,
  ToggleSetting,
} from "@/components/settings/settings-form";
import { useSaveShortcut } from "@/components/settings/use-save-shortcut";

const ITEM = SETTINGS_GROUPS.flatMap((g) => g.tabs).find((t) => t.legacyPath === "/settings/llm")!;

/** A price row as edited. Numbers stay strings so a half-typed `0.` or an
 * intentionally blank cached price isn't coerced under the cursor. */
type PriceRow = { model: string; input: string; cachedInput: string; output: string };

type Draft = {
  llm: Omit<ServerSettings["llm"], "prices">;
  prices: PriceRow[];
};

function priceRowsOf(prices: ServerSettings["llm"]["prices"] | undefined): PriceRow[] {
  return Object.entries(prices ?? {}).map(([model, p]) => ({
    model,
    input: String(p.input ?? 0),
    cachedInput: p.cachedInput == null ? "" : String(p.cachedInput),
    output: String(p.output ?? 0),
  }));
}

function draftOf(settings: ServerSettings): Draft {
  const { prices, ...llm } = settings.llm;
  return { llm: { ...llm, priceVersion: llm.priceVersion ?? "" }, prices: priceRowsOf(prices) };
}

/** Only send the api key when one was typed — an empty box means "keep
 * what is stored", same convention as `smtp.password`/`s3.secret`. Prices
 * deleted from the table are sent as `null`: settings PATCH is a deep merge,
 * so a key is only dropped by being told to be. */
function payloadOf(draft: Draft, saved: ServerSettings) {
  const { apiKey, ...rest } = draft.llm;
  const prices: Record<string, { input: number; output: number; cachedInput: number | null } | null> = {};
  for (const model of Object.keys(saved.llm.prices ?? {})) prices[model] = null;
  for (const row of draft.prices) {
    prices[row.model.trim()] = {
      input: Number(row.input),
      output: Number(row.output),
      cachedInput: row.cachedInput.trim() === "" ? null : Number(row.cachedInput),
    };
  }
  return { llm: { ...rest, ...(apiKey ? { apiKey } : {}), prices } };
}

function isDirty(draft: Draft, settings: ServerSettings): boolean {
  return JSON.stringify(draft) !== JSON.stringify(draftOf(settings));
}

const isNonNegative = (value: string) => value.trim() !== "" && Number.isFinite(Number(value)) && Number(value) >= 0;

type Errors = { fields: Record<string, string>; rows: Record<number, string>; list: string[] };

function validate(draft: Draft): Errors {
  const fields: Record<string, string> = {};
  const rows: Record<number, string> = {};
  const { enabled, baseUrl, model } = draft.llm;
  if (enabled) {
    try {
      new URL(baseUrl);
    } catch {
      fields.baseUrl = "Must be a full URL, e.g. https://api.openai.com/v1";
    }
    if (!model.trim()) fields.model = "A default model is required while the provider is enabled";
  }
  const seen = new Set<string>();
  draft.prices.forEach((row, i) => {
    const name = row.model.trim();
    if (!name) rows[i] = "Enter the model name exactly as the provider reports it";
    else if (seen.has(name)) rows[i] = `"${name}" is listed twice`;
    else if (!isNonNegative(row.input) || !isNonNegative(row.output)) rows[i] = "Input and output prices must be numbers ≥ 0";
    else if (row.cachedInput.trim() !== "" && !isNonNegative(row.cachedInput)) rows[i] = "Cached input must be a number ≥ 0, or blank";
    seen.add(name);
  });
  const list = [...Object.values(fields), ...Object.values(rows)];
  return { fields, rows, list };
}

/** `$0.0018`, never scientific notation, and "<$0.0001" rather than a lying "$0.0000". */
function formatUsd(value: number): string {
  if (value === 0) return "$0.00";
  if (value < 0.0001) return "<$0.0001";
  return value.toLocaleString(undefined, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: value < 1 ? 4 : 2,
    maximumFractionDigits: value < 1 ? 4 : 2,
  });
}

type GroupBy = "model" | "surface" | "purpose" | "actor" | "day";
type Range = "24h" | "7d" | "30d" | "all";

const GROUPS: { value: GroupBy; label: string }[] = [
  { value: "model", label: "Model" },
  { value: "surface", label: "Surface" },
  { value: "purpose", label: "Purpose" },
  { value: "actor", label: "Actor" },
  { value: "day", label: "Day" },
];

const RANGES: { value: Range; label: string; hours: number | null }[] = [
  { value: "24h", label: "Last 24 hours", hours: 24 },
  { value: "7d", label: "Last 7 days", hours: 24 * 7 },
  { value: "30d", label: "Last 30 days", hours: 24 * 30 },
  { value: "all", label: "All time", hours: null },
];

type UsageSummary = {
  items: { key: string; calls: number; promptTokens: number; completionTokens: number; costUsd: number }[];
};

/** `created` is stored as `YYYY-MM-DD HH:MM:SS.sssZ`; the bound has to use the
 * same shape or same-day comparisons go wrong (`T` sorts after a space). */
function sinceParam(hours: number | null): string | undefined {
  if (hours == null) return undefined;
  return new Date(Date.now() - hours * 3_600_000).toISOString().replace("T", " ");
}

/** Spend and token totals from the `_llm_usage` ledger, grouped and ranged
 * from `GET /api/llm/usage/summary`. */
function UsageSummarySection({ pricedModels, onPriceModel }: { pricedModels: Set<string>; onPriceModel: (model: string) => void }) {
  const [groupBy, setGroupBy] = useState<GroupBy>("model");
  const [range, setRange] = useState<Range>("30d");
  const hours = RANGES.find((r) => r.value === range)!.hours;

  const { data, isPending, isError, error, refetch } = useQuery({
    queryKey: ["llm-usage", groupBy, range],
    queryFn: () =>
      cb.send<UsageSummary>("/api/llm/usage/summary", {
        method: "GET",
        query: { groupBy, from: sinceParam(hours) },
      }),
    placeholderData: (previous) => previous,
  });

  const items = useMemo(() => data?.items ?? [], [data]);
  const totals = useMemo(
    () =>
      items.reduce(
        (acc, i) => ({
          calls: acc.calls + i.calls,
          tokens: acc.tokens + i.promptTokens + i.completionTokens,
          cost: acc.cost + i.costUsd,
        }),
        { calls: 0, tokens: 0, cost: 0 },
      ),
    [items],
  );
  const maxCost = Math.max(0, ...items.map((i) => i.costUsd));

  return (
    <SettingsSection
      title="Usage"
      description="Calls, tokens and spend recorded in the usage ledger — by the built-in gateway and by anything that reports through POST /api/llm/usage."
      action={
        <div className="flex items-center gap-2">
          <Select value={range} onValueChange={(v) => setRange(v as Range)}>
            <SelectTrigger size="sm" aria-label="Time range" className="h-control-sm w-36">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {RANGES.map((r) => (
                <SelectItem key={r.value} value={r.value}>
                  {r.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select value={groupBy} onValueChange={(v) => setGroupBy(v as GroupBy)}>
            <SelectTrigger size="sm" aria-label="Group by" className="h-control-sm w-32">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {GROUPS.map((g) => (
                <SelectItem key={g.value} value={g.value}>
                  By {g.label.toLowerCase()}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      }
    >
      <div className="grid grid-cols-3 gap-2">
        <StatCard index={0} label="Calls" value={totals.calls.toLocaleString()} loading={isPending} />
        <StatCard index={1} label="Tokens" value={totals.tokens.toLocaleString()} loading={isPending} hint="prompt + completion" />
        <StatCard index={2} label="Spend" value={formatUsd(totals.cost)} loading={isPending} hint="priced calls only" />
      </div>

      {isPending ? (
        <ListSkeleton rows={3} />
      ) : isError ? (
        <ErrorState title="Couldn't load usage" error={error} onRetry={() => void refetch()} compact />
      ) : items.length === 0 ? (
        <EmptyState
          compact
          icon={<Coins />}
          title="No usage recorded in this range"
          description="Calls through POST /api/llm/chat are logged automatically. Report your own provider calls with POST /api/llm/usage (or cb.llm.logUsage in the SDK) to see their cost here."
        />
      ) : (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{GROUPS.find((g) => g.value === groupBy)!.label}</TableHead>
                <TableHead className="w-20 text-right">Calls</TableHead>
                <TableHead className="w-28 text-right">Prompt</TableHead>
                <TableHead className="w-28 text-right">Completion</TableHead>
                <TableHead className="w-36 text-right">Cost</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {items.map((item) => (
                <TableRow key={item.key}>
                  <TableCell className="font-mono text-xs">
                    <span className="inline-flex items-center gap-2">
                      {item.key || <span className="text-muted-foreground">(none)</span>}
                      {groupBy === "model" && item.key && !pricedModels.has(item.key) ? (
                        <button type="button" onClick={() => onPriceModel(item.key)} title="Add a price for this model">
                          <StatusBadge tone="warning">Unpriced · add</StatusBadge>
                        </button>
                      ) : null}
                    </span>
                  </TableCell>
                  <TableCell className="font-tabular text-right text-xs">{item.calls.toLocaleString()}</TableCell>
                  <TableCell className="font-tabular text-right text-xs text-muted-foreground">
                    {item.promptTokens.toLocaleString()}
                  </TableCell>
                  <TableCell className="font-tabular text-right text-xs text-muted-foreground">
                    {item.completionTokens.toLocaleString()}
                  </TableCell>
                  <TableCell className="text-right">
                    <span className="inline-flex items-center justify-end gap-2">
                      <span aria-hidden className="relative h-1 w-12 overflow-hidden rounded-full bg-muted">
                        <span
                          className="absolute inset-y-0 left-0 rounded-full bg-primary/70"
                          style={{ width: `${maxCost > 0 ? Math.max(4, (item.costUsd / maxCost) * 100) : 0}%` }}
                        />
                      </span>
                      <span className="font-tabular w-16 text-right text-xs">{formatUsd(item.costUsd)}</span>
                    </span>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </SettingsSection>
  );
}

function LlmSkeleton() {
  return (
    <SettingsPage title={ITEM.label} description={ITEM.description} width="form">
      <Skeleton className="h-64 w-full" />
      <Skeleton className="h-40 w-full" />
    </SettingsPage>
  );
}

/**
 * `Settings.llm` (`crates/core/src/settings.rs`), consumed by
 * `POST /api/llm/chat` (`crates/server/src/llm.rs`) — the same provider
 * config a vector field's auto-embedding falls back to when its own
 * `embedding.provider` isn't `"echo"`. Disabled means requests are
 * answered by a deterministic, network-free echo provider instead of a
 * real model. Also owns the price table that turns recorded tokens into
 * `costUsd`, and a spend summary over the usage ledger.
 */
export function LlmPage() {
  const { data: settings, isPending } = useSettings();
  const save = useSettingsMutation();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [seedKey, setSeedKey] = useState<ServerSettings | undefined>(undefined);

  if (settings && (draft === null || (seedKey !== settings && !isDirty(draft, settings)))) {
    setSeedKey(settings);
    setDraft(draftOf(settings));
  }

  const errors = useMemo(() => (draft ? validate(draft) : null), [draft]);
  const dirty = draft && settings ? isDirty(draft, settings) : false;

  function submit() {
    if (!draft || !settings || !errors || errors.list.length > 0) return;
    save.mutate(payloadOf(draft, settings), {
      onSuccess: () => {
        toast.success("LLM settings saved");
        // The secret was consumed; clear the box so it reads as "stored".
        setDraft((d) => (d ? { ...d, llm: { ...d.llm, apiKey: "" } } : d));
      },
      onError: (error) => {
        const failure = describeFailure(error);
        toast.error("Couldn't save LLM settings", { description: failure.serverMessage || failure.detail });
      },
    });
  }
  useSaveShortcut({ dirty, canSave: !!errors && errors.list.length === 0 && !save.isPending, onSave: submit });

  if (isPending || !draft || !settings || !errors) return <LlmSkeleton />;

  function patchLlm(next: Partial<Draft["llm"]>) {
    setDraft((d) => (d ? { ...d, llm: { ...d.llm, ...next } } : d));
  }
  function patchRow(index: number, next: Partial<PriceRow>) {
    setDraft((d) => (d ? { ...d, prices: d.prices.map((row, i) => (i === index ? { ...row, ...next } : row)) } : d));
  }
  function addRow(model = "") {
    setDraft((d) => {
      if (!d) return d;
      if (model && d.prices.some((r) => r.model === model)) return d;
      return { ...d, prices: [...d.prices, { model, input: "0", cachedInput: "", output: "0" }] };
    });
    // The price table sits below the provider block; bring it into view for
    // the "Unpriced · add" shortcut in the usage table.
    requestAnimationFrame(() => document.getElementById("llm-prices")?.scrollIntoView({ block: "center", behavior: "smooth" }));
  }
  function removeRow(index: number) {
    setDraft((d) => (d ? { ...d, prices: d.prices.filter((_, i) => i !== index) } : d));
  }

  const pricedModels = new Set(Object.keys(settings.llm.prices ?? {}));

  return (
    <SettingsPage
      title={ITEM.label}
      description={ITEM.description}
      width="form"
      action={
        <StatusBadge tone={settings.llm.enabled ? "success" : "neutral"}>
          {settings.llm.enabled ? "Provider enabled" : "Echo provider"}
        </StatusBadge>
      }
    >
      <SettingsSection
        title="Provider"
        description="Backs POST /api/llm/chat and any vector field whose auto-embedding provider isn't the echo test provider."
      >
        <SettingRow
          label="Enabled"
          htmlFor="llm-enabled"
          help="Off answers with a deterministic, network-free echo provider instead of a real model — handy for tests and local dev."
        >
          <ToggleSetting
            id="llm-enabled"
            checked={draft.llm.enabled}
            onChange={(enabled) => patchLlm({ enabled })}
            label={draft.llm.enabled ? "Using the configured provider" : "Using the echo provider"}
          />
        </SettingRow>
        <SettingRow
          label="Provider"
          htmlFor="llm-provider"
          help={'"openai" (or any OpenAI-compatible /chat/completions API, e.g. a local Ollama instance) is the only real provider today.'}
        >
          <TextSetting
            id="llm-provider"
            value={draft.llm.provider}
            onChange={(provider) => patchLlm({ provider })}
            placeholder="openai"
            mono
          />
        </SettingRow>
        <SettingRow
          label="Base URL"
          htmlFor="llm-base-url"
          help="The API root, without /chat/completions."
          error={errors.fields.baseUrl}
        >
          <TextSetting
            id="llm-base-url"
            value={draft.llm.baseUrl}
            onChange={(baseUrl) => patchLlm({ baseUrl })}
            placeholder="https://api.openai.com/v1"
            mono
            invalid={!!errors.fields.baseUrl}
          />
        </SettingRow>
        <SettingRow
          label="API key"
          htmlFor="llm-api-key"
          help="Never sent back by the server. Leave blank to keep the stored one."
        >
          <SecretSetting
            id="llm-api-key"
            value={draft.llm.apiKey ?? ""}
            onChange={(apiKey) => patchLlm({ apiKey })}
            storedHint="•••••••• (unchanged)"
          />
        </SettingRow>
        <SettingRow
          label="Default model"
          htmlFor="llm-model"
          help="Used when a request doesn't name one."
          error={errors.fields.model}
        >
          <TextSetting
            id="llm-model"
            value={draft.llm.model}
            onChange={(model) => patchLlm({ model })}
            placeholder="gpt-4o-mini"
            mono
            invalid={!!errors.fields.model}
          />
        </SettingRow>
      </SettingsSection>

      <section id="llm-prices" className="scroll-mt-20">
        <SettingsSection
          title="Model prices"
          description="USD per 1M tokens, keyed by the exact model name. Every usage row is priced from this table when it is recorded — editing it doesn't reprice history."
          action={
            <Button type="button" variant="outline" size="sm" onClick={() => addRow()}>
              <Plus />
              Add model
            </Button>
          }
        >
          <SettingRow
            label="Price version"
            htmlFor="llm-price-version"
            help="A free-form label stamped on each usage row (e.g. 2026-10). Bump it when you change prices so old and new rows can be told apart."
          >
            <TextSetting
              id="llm-price-version"
              value={draft.llm.priceVersion}
              onChange={(priceVersion) => patchLlm({ priceVersion })}
              placeholder="2026-10"
              mono
            />
          </SettingRow>

          {draft.prices.length === 0 ? (
            <EmptyState
              compact
              icon={<Coins />}
              title="No prices yet"
              description="Without a price, a model's calls are recorded with their token counts but no cost. Add the models you use to see spend."
              action={
                <Button type="button" size="sm" variant="outline" onClick={() => addRow(draft.llm.model.trim())}>
                  <Plus />
                  {draft.llm.model.trim() ? `Add ${draft.llm.model.trim()}` : "Add model"}
                </Button>
              }
            />
          ) : (
            <div className="flex flex-col gap-1.5">
              <div
                aria-hidden
                className="hidden grid-cols-[minmax(0,1fr)_6.5rem_6.5rem_6.5rem_auto] gap-2 px-0.5 text-xs text-muted-foreground sm:grid"
              >
                <span>Model</span>
                <span>Input</span>
                <span>Cached input</span>
                <span>Output</span>
                <span className="w-control-sm" />
              </div>
              {draft.prices.map((row, i) => (
                <div key={i} className="flex flex-col gap-1">
                  <div className="grid grid-cols-2 items-center gap-2 sm:grid-cols-[minmax(0,1fr)_6.5rem_6.5rem_6.5rem_auto]">
                    <Input
                      value={row.model}
                      onChange={(e) => patchRow(i, { model: e.target.value })}
                      placeholder="gpt-4o-mini"
                      aria-label={`Price ${i + 1} model`}
                      aria-invalid={errors.rows[i] ? true : undefined}
                      className="col-span-2 h-control-sm font-mono text-xs sm:col-span-1"
                    />
                    <Input
                      inputMode="decimal"
                      value={row.input}
                      onChange={(e) => patchRow(i, { input: e.target.value })}
                      aria-label={`${row.model || `Price ${i + 1}`} input price per million tokens`}
                      aria-invalid={!isNonNegative(row.input) || undefined}
                      placeholder="input"
                      className="h-control-sm font-tabular text-xs"
                    />
                    <Input
                      inputMode="decimal"
                      value={row.cachedInput}
                      onChange={(e) => patchRow(i, { cachedInput: e.target.value })}
                      aria-label={`${row.model || `Price ${i + 1}`} cached input price per million tokens`}
                      placeholder="= input"
                      className="h-control-sm font-tabular text-xs"
                    />
                    <Input
                      inputMode="decimal"
                      value={row.output}
                      onChange={(e) => patchRow(i, { output: e.target.value })}
                      aria-label={`${row.model || `Price ${i + 1}`} output price per million tokens`}
                      aria-invalid={!isNonNegative(row.output) || undefined}
                      placeholder="output"
                      className="h-control-sm font-tabular text-xs"
                    />
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      aria-label={`Remove price for ${row.model || `row ${i + 1}`}`}
                      onClick={() => removeRow(i)}
                    >
                      <Trash2 className="text-destructive" />
                    </Button>
                  </div>
                  {errors.rows[i] ? (
                    <p role="alert" className={cn("text-xs text-destructive")}>
                      {errors.rows[i]}
                    </p>
                  ) : null}
                </div>
              ))}
              <p className="text-xs text-muted-foreground">
                Cached input defaults to the input price when left blank. Cost = (non-cached prompt × input + cached ×
                cached input + completion × output) ÷ 1,000,000.
              </p>
            </div>
          )}
        </SettingsSection>
      </section>

      <UsageSummarySection pricedModels={pricedModels} onPriceModel={addRow} />

      <SettingsSaveBar
        dirty={dirty}
        pending={save.isPending}
        errors={errors.list}
        onSave={submit}
        onReset={() => setDraft(draftOf(settings))}
      />
    </SettingsPage>
  );
}
