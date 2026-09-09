import { useMemo, useState } from "react";
import { Check, Copy } from "lucide-react";
import type { CollectionModel } from "@cratebase/client";
import { cn } from "@/lib/utils";
import { buildDocEndpoints, type DocEndpoint, type Method } from "@/lib/api-docs";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { Badge } from "@/components/ui/badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

/**
 * The full-page counterpart to the collapsed preview in the schema editor —
 * this is PocketBase's own "API Preview" tab, one level up: not a form
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

const RULE_BADGE_CLASS: Record<DocEndpoint["rule"]["tone"], string> = {
  public: "text-success",
  restricted: "text-warning",
  locked: "text-destructive",
  fixed: "text-muted-foreground",
};

type Lang = "curl" | "js";

function CodeBlock({ code }: { code: string }) {
  const { copy, status } = useCopyToClipboard();
  return (
    <div className="relative">
      <pre className="max-h-96 overflow-auto rounded-lg border border-border bg-surface-sunken px-3 py-2.5 font-mono text-xs leading-relaxed text-foreground">
        {code}
      </pre>
      <button
        type="button"
        onClick={() => void copy(code)}
        aria-label={status === "copied" ? "Copied" : "Copy snippet"}
        className="absolute right-1.5 top-1.5 grid size-control-xs place-items-center rounded border border-border bg-background text-muted-foreground transition-colors hover:text-foreground"
      >
        {status === "copied" ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
      </button>
    </div>
  );
}

function EndpointPanel({ endpoint, lang }: { endpoint: DocEndpoint; lang: Lang }) {
  const rule = endpoint.rule;
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <span className={cn("font-mono text-xs font-semibold", METHOD_CLASS[endpoint.method])}>
          {endpoint.method}
        </span>
        <code className="min-w-0 truncate rounded bg-surface-sunken px-1.5 py-0.5 font-mono text-xs text-foreground">
          {endpoint.path}
        </code>
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

      <div className="flex flex-col gap-1">
        <span className="text-xs font-medium text-muted-foreground">Request</span>
        <CodeBlock code={lang === "curl" ? endpoint.curl : endpoint.js} />
      </div>

      <div className="flex flex-col gap-1">
        <span className="text-xs font-medium text-muted-foreground">
          Response <span className="font-mono">{endpoint.responseStatus}</span>
        </span>
        <CodeBlock
          code={
            endpoint.responseBody === undefined
              ? "// no body"
              : JSON.stringify(endpoint.responseBody, null, 2)
          }
        />
      </div>
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
  const activeId = endpoints.some((e) => e.id === active) ? active : (endpoints[0]?.id ?? "list");

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-page">
      <div className="flex flex-col gap-1">
        <h2 className="text-sm font-semibold text-foreground">API reference — {collection.name}</h2>
        <p className="text-xs text-muted-foreground">
          Every endpoint this collection exposes, generated from its own fields and rules. Snippets assume{" "}
          <code className="font-mono">{'const cb = createClient("{origin}")'}</code> from{" "}
          <code className="font-mono">@cratebase/client</code>.
        </p>
      </div>

      <Tabs value={lang} onValueChange={(v) => setLang(v as Lang)} className="w-fit gap-0">
        <TabsList>
          <TabsTrigger value="curl" className="text-xs">
            cURL
          </TabsTrigger>
          <TabsTrigger value="js" className="text-xs">
            JavaScript
          </TabsTrigger>
        </TabsList>
      </Tabs>

      <Tabs value={activeId} onValueChange={setActive} className="gap-4">
        <TabsList className="flex-wrap">
          {endpoints.map((endpoint) => (
            <TabsTrigger key={endpoint.id} value={endpoint.id} className="text-xs">
              {endpoint.label}
            </TabsTrigger>
          ))}
        </TabsList>

        {endpoints.map((endpoint) => (
          <TabsContent key={endpoint.id} value={endpoint.id}>
            <EndpointPanel endpoint={endpoint} lang={lang} />
          </TabsContent>
        ))}
      </Tabs>
    </div>
  );
}
