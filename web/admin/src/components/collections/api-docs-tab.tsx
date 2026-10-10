import { useMemo, useState } from "react";
import { Check, Copy, FileCode2 } from "lucide-react";
import type { CollectionModel } from "@cratebase/client";
import { toast } from "sonner";
import { cb, superuserAuth } from "@/lib/api";
import { cn } from "@/lib/utils";
import { buildDocEndpoints, type DocEndpoint, type Method } from "@/lib/api-docs";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";

/** `GET /api/typegen` requires a superuser bearer token, which a plain
 * `<a href>` navigation can't attach — same pattern
 * `settings/backups-page.tsx`'s `downloadBackup` uses: fetch with the
 * `Authorization` header, then hand the browser a blob URL to save. */
async function downloadTypes(): Promise<void> {
  const headers: Record<string, string> = {};
  if (superuserAuth.token) headers.authorization = `Bearer ${superuserAuth.token}`;
  const response = await fetch(cb.buildURL("/api/typegen"), { headers });
  if (!response.ok) {
    throw new Error(`download failed with status ${response.status}`);
  }
  const blob = await response.blob();
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = "cratebase-types.d.ts";
  link.click();
  URL.revokeObjectURL(url);
}

/**
 * The full-page counterpart to the collapsed preview in the schema editor —
 * this is an "API Preview" tab, one level up: not a form
 * sidebar but a standalone reference someone building against this
 * collection can leave open in a tab of its own, the way `collection.tsx`'s
 * "records" and "settings" tabs already work.
 *
 * Everything rendered here is derived from the live `CollectionModel` —
 * field types drive the example payloads, `listRule`/`viewRule`/etc. drive
 * the "who can call this" line — so it never drifts from what the server
 * will actually accept, the way a hand-written docs page would.
 */

const METHOD_CLASS: Record<Method, string> = {
  GET: "text-info",
  POST: "text-success",
  PATCH: "text-warning",
  DELETE: "text-destructive",
};

const METHOD_LABEL: Record<Method, string> = { GET: "GET", POST: "POST", PATCH: "PATCH", DELETE: "DEL" };

const RULE_BADGE_CLASS: Record<DocEndpoint["rule"]["tone"], string> = {
  public: "text-success",
  restricted: "text-warning",
  locked: "text-destructive",
  fixed: "text-muted-foreground",
};

type Lang = "curl" | "js";

/** A snippet with a title bar: what it is on the left, optional extras
 * (the language switch) and a labelled copy button on the right. */
function CodeBlock({ code, title, extra }: { code: string; title: string; extra?: React.ReactNode }) {
  const { copy, status } = useCopyToClipboard();
  return (
    <div className="overflow-hidden rounded-lg border border-border bg-surface-sunken">
      <div className="flex items-center justify-between gap-2 border-b border-border px-3 py-1">
        <span className="text-xs font-medium text-muted-foreground">{title}</span>
        <div className="flex items-center gap-1.5">
          {extra}
          <Button
            type="button"
            variant="ghost"
            size="xs"
            onClick={() => void copy(code)}
            aria-label={status === "copied" ? "Copied" : `Copy ${title.toLowerCase()}`}
            className="gap-1 text-muted-foreground hover:text-foreground"
          >
            {status === "copied" ? <Check className="size-3.5 text-success" /> : <Copy className="size-3.5" />}
            {status === "copied" ? "Copied" : "Copy"}
          </Button>
        </div>
      </div>
      <pre className="max-h-96 overflow-auto px-3 py-2.5 font-mono text-xs leading-relaxed text-foreground">{code}</pre>
    </div>
  );
}

function EndpointPanel({
  endpoint,
  lang,
  onLangChange,
}: {
  endpoint: DocEndpoint;
  lang: Lang;
  onLangChange: (lang: Lang) => void;
}) {
  const rule = endpoint.rule;
  const { copy, status } = useCopyToClipboard();
  return (
    <div key={endpoint.id} className="animate-rise flex min-w-0 flex-col gap-4">
      <div className="flex min-w-0 items-center gap-2 rounded-lg border border-border bg-card px-3 py-2">
        <span className={cn("font-mono text-xs font-semibold", METHOD_CLASS[endpoint.method])}>
          {endpoint.method}
        </span>
        <code className="min-w-0 flex-1 truncate font-mono text-xs text-foreground" title={endpoint.path}>
          {endpoint.path}
        </code>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label={status === "copied" ? "Copied" : "Copy path"}
          onClick={() => void copy(endpoint.path)}
          className="text-muted-foreground hover:text-foreground"
        >
          {status === "copied" ? <Check className="size-3.5 text-success" /> : <Copy className="size-3.5" />}
        </Button>
      </div>

      <div className="flex flex-col gap-1">
        <span className="text-xs font-medium text-muted-foreground">Who can call this</span>
        <div className="flex flex-wrap items-center gap-2">
          {rule.field ? (
            <Badge variant="outline" className="font-mono font-normal text-muted-foreground">
              {rule.field}
              {rule.value === null ? ": null" : rule.value === "" ? ': ""' : `: "${rule.value}"`}
            </Badge>
          ) : null}
          <span className={cn("text-xs", RULE_BADGE_CLASS[rule.tone])}>{rule.summary}</span>
        </div>
      </div>

      {endpoint.headers.length > 0 ? (
        <div className="flex flex-col gap-1">
          <span className="text-xs font-medium text-muted-foreground">Headers</span>
          <div className="flex flex-col gap-1 rounded-lg border border-border bg-surface-sunken px-3 py-2">
            {endpoint.headers.map((h) => (
              <div key={h.name} className="flex items-center gap-2 font-mono text-xs">
                <span className="text-foreground">
                  {h.name}: {h.value}
                </span>
                {h.required ? (
                  <Badge variant="outline" className="font-sans text-2xs font-normal text-warning">
                    required
                  </Badge>
                ) : null}
              </div>
            ))}
          </div>
        </div>
      ) : null}

      <CodeBlock
        title="Request"
        code={lang === "curl" ? endpoint.curl : endpoint.js}
        extra={
          <Tabs value={lang} onValueChange={(v) => onLangChange(v as Lang)} className="gap-0">
            <TabsList className="h-control-xs">
              <TabsTrigger value="curl" className="px-2 text-xs">
                cURL
              </TabsTrigger>
              <TabsTrigger value="js" className="px-2 text-xs">
                JavaScript
              </TabsTrigger>
            </TabsList>
          </Tabs>
        }
      />

      <CodeBlock
        title={`Response ${endpoint.responseStatus}`}
        code={
          endpoint.responseBody === undefined ? "// no body" : JSON.stringify(endpoint.responseBody, null, 2)
        }
      />
    </div>
  );
}

export function ApiDocsTab({ collection }: { collection: CollectionModel }) {
  const origin = typeof window === "undefined" ? "https://your-instance.example.com" : window.location.origin;
  const endpoints = useMemo(() => buildDocEndpoints(collection, origin), [collection, origin]);
  const [active, setActive] = useState(endpoints[0]?.id ?? "list");
  const [lang, setLang] = useState<Lang>("curl");

  // A different collection can drop the previously-active endpoint id
  // (auth endpoints only exist on auth collections) — fall back to the
  // first one rather than rendering an empty panel.
  const activeEndpoint = endpoints.find((e) => e.id === active) ?? endpoints[0];

  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col gap-4 p-3 sm:p-page">
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2">
        <div className="flex min-w-0 flex-col gap-0.5">
          <h2 className="text-base font-semibold">API reference</h2>
          <p className="max-w-measure text-sm text-pretty text-muted-foreground">
            Generated from <span className="font-mono">{collection.name}</span>'s fields and rules. Snippets assume{" "}
            <code className="font-mono">{`const cb = createClient("${origin}")`}</code> from{" "}
            <code className="font-mono">@cratebase/client</code>.
          </p>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="gap-1.5"
          onClick={() =>
            void downloadTypes().catch((error: unknown) =>
              toast.error(`Download failed: ${error instanceof Error ? error.message : String(error)}`),
            )
          }
        >
          <FileCode2 className="size-3.5" />
          Download TypeScript types
        </Button>
      </div>

      <div className="grid gap-4 md:grid-cols-[13rem_minmax(0,1fr)]">
        <nav
          aria-label="Endpoints"
          className="flex gap-1 overflow-x-auto md:sticky md:top-0 md:flex-col md:self-start md:overflow-visible"
        >
          {endpoints.map((endpoint) => {
            const selected = endpoint.id === activeEndpoint?.id;
            return (
              <button
                key={endpoint.id}
                type="button"
                aria-current={selected ? "true" : undefined}
                onClick={() => setActive(endpoint.id)}
                className={cn(
                  "flex h-control-md shrink-0 items-center gap-2 rounded-md px-2 text-left text-sm transition-colors duration-fast",
                  selected
                    ? "bg-brand-soft font-medium text-foreground"
                    : "text-muted-foreground hover:bg-accent hover:text-foreground",
                )}
              >
                <span className={cn("w-8 shrink-0 font-mono text-2xs font-semibold", METHOD_CLASS[endpoint.method])}>
                  {METHOD_LABEL[endpoint.method]}
                </span>
                <span className="truncate">{endpoint.label}</span>
              </button>
            );
          })}
        </nav>

        {activeEndpoint ? <EndpointPanel endpoint={activeEndpoint} lang={lang} onLangChange={setLang} /> : null}
      </div>
    </div>
  );
}
