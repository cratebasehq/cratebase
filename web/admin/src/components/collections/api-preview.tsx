import { useMemo, useState } from "react";
import { Check, Copy } from "lucide-react";
import type { CollectionModel } from "pocketbase";
import { cn } from "@/lib/utils";
import { userFields } from "@/lib/field-types";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { Badge } from "@/components/ui/badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

/**
 * The per-collection API reference PocketBase's own dashboard ships, cut
 * down to what a backend actually reaches for: the endpoint, the rule that
 * guards it, and a runnable snippet against the `pocketbase` JS client.
 *
 * The schema is already in hand on this screen, so the example payloads are
 * this collection's real fields rather than a generic `{...}`.
 */

type Method = "GET" | "POST" | "PATCH" | "DELETE";

interface Endpoint {
  id: string;
  label: string;
  method: Method;
  path: string;
  /** Which API rule decides whether the call is allowed. */
  rule: string;
  ruleValue: string | null | undefined;
  snippet: string;
}

const METHOD_CLASS: Record<Method, string> = {
  GET: "text-info",
  POST: "text-success",
  PATCH: "text-warning",
  DELETE: "text-destructive",
};

/** A plausible value for each field type, so the create/update snippets are
 * copy-run-able rather than a shape you still have to fill in. */
function sampleValue(field: { type: string; name: string; values?: unknown; maxSelect?: unknown }): string {
  const multi = Number(field.maxSelect ?? 1) > 1;
  switch (field.type) {
    case "number":
      return "42";
    case "bool":
      return "true";
    case "json":
      return `{ note: "anything JSON-serialisable" }`;
    case "date":
      return `"2026-01-31 12:00:00.000Z"`;
    case "email":
      return `"someone@example.com"`;
    case "url":
      return `"https://example.com"`;
    case "select": {
      const values = (field.values as string[] | undefined) ?? [];
      const first = values[0] ?? "value";
      return multi ? `["${first}"]` : `"${first}"`;
    }
    case "relation":
      return multi ? `["RELATED_RECORD_ID"]` : `"RELATED_RECORD_ID"`;
    case "file":
      return multi ? `[file1, file2] /* File objects */` : `file /* a File object */`;
    default:
      return `"${field.name}"`;
  }
}

function buildEndpoints(collection: CollectionModel): Endpoint[] {
  const name = collection.name;
  const fields = userFields(collection).filter((f) => f.type !== "autodate");
  const body = fields
    .slice(0, 6)
    .map((f) => `  ${f.name}: ${sampleValue(f)},`)
    .join("\n");
  const relation = fields.find((f) => f.type === "relation");
  const expandHint = relation ? `, expand: "${relation.name}"` : "";

  const list: Endpoint = {
    id: "list",
    label: "List / search",
    method: "GET",
    path: `/api/collections/${name}/records`,
    rule: "listRule",
    ruleValue: collection.listRule,
    snippet: `const page = await pb.collection("${name}").getList(1, 50, {
  filter: 'created >= @todayStart',
  sort: "-created"${expandHint},
  skipTotal: true, // drops the COUNT(*)
});`,
  };

  const view: Endpoint = {
    id: "view",
    label: "View one",
    method: "GET",
    path: `/api/collections/${name}/records/:id`,
    rule: "viewRule",
    ruleValue: collection.viewRule,
    snippet: `const record = await pb.collection("${name}").getOne("RECORD_ID"${
      relation ? `, { expand: "${relation.name}" }` : ""
    });`,
  };

  const create: Endpoint = {
    id: "create",
    label: "Create",
    method: "POST",
    path: `/api/collections/${name}/records`,
    rule: "createRule",
    ruleValue: collection.createRule,
    snippet: `const record = await pb.collection("${name}").create({
${body || "  // no user-defined fields yet"}
});`,
  };

  const update: Endpoint = {
    id: "update",
    label: "Update",
    method: "PATCH",
    path: `/api/collections/${name}/records/:id`,
    rule: "updateRule",
    ruleValue: collection.updateRule,
    snippet: `const record = await pb.collection("${name}").update("RECORD_ID", {
${body || "  // no user-defined fields yet"}
});`,
  };

  const remove: Endpoint = {
    id: "delete",
    label: "Delete",
    method: "DELETE",
    path: `/api/collections/${name}/records/:id`,
    rule: "deleteRule",
    ruleValue: collection.deleteRule,
    snippet: `await pb.collection("${name}").delete("RECORD_ID");`,
  };

  const realtime: Endpoint = {
    id: "realtime",
    label: "Subscribe",
    method: "GET",
    path: `/api/realtime`,
    rule: "listRule",
    ruleValue: collection.listRule,
    snippet: `const unsubscribe = await pb.collection("${name}").subscribe("*", (e) => {
  console.log(e.action, e.record); // "create" | "update" | "delete"
});`,
  };

  const batch: Endpoint = {
    id: "batch",
    label: "Batch",
    method: "POST",
    path: `/api/batch`,
    rule: "createRule / updateRule / deleteRule",
    ruleValue: collection.createRule,
    snippet: `const batch = pb.createBatch();
batch.collection("${name}").create({ /* … */ });
batch.collection("${name}").delete("RECORD_ID");
await batch.send(); // one transaction — all of it, or none`,
  };

  const base = [list, view, create, update, remove, realtime, batch];

  if (collection.type !== "auth") return base;

  const identity = collection.passwordAuth?.identityFields?.[0] ?? "email";
  const auth: Endpoint = {
    id: "auth",
    label: "Auth",
    method: "POST",
    path: `/api/collections/${name}/auth-with-password`,
    rule: "— always allowed for this collection",
    ruleValue: undefined,
    snippet: `const auth = await pb.collection("${name}").authWithPassword(
  "${identity === "email" ? "someone@example.com" : "someuser"}",
  "a-password",
);
pb.authStore.isValid; // true`,
  };
  return [auth, ...base];
}

function ruleSummary(endpoint: Endpoint): { text: string; tone: "open" | "auth" | "locked" | "none" } {
  if (endpoint.ruleValue === undefined) return { text: endpoint.rule, tone: "none" };
  if (endpoint.ruleValue === null) return { text: "superusers only", tone: "locked" };
  if (endpoint.ruleValue === "") return { text: "public", tone: "open" };
  return { text: endpoint.ruleValue, tone: "auth" };
}

function Snippet({ code }: { code: string }) {
  const { copy, status } = useCopyToClipboard();
  return (
    <div className="relative">
      <pre className="max-h-72 overflow-auto rounded-lg border border-border bg-surface-sunken px-3 py-2.5 font-mono text-xs leading-relaxed text-foreground">
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

export function ApiPreview({ collection }: { collection: CollectionModel }) {
  const endpoints = useMemo(() => buildEndpoints(collection), [collection]);
  const [active, setActive] = useState(endpoints[0]?.id ?? "list");

  return (
    <section className="flex flex-col gap-2">
      <div className="flex flex-col">
        <span className="text-sm font-medium text-foreground">API preview</span>
        <span className="text-xs text-muted-foreground">
          Every endpoint this collection exposes, and which rule guards it.
        </span>
      </div>

      <Tabs value={active} onValueChange={setActive} className="gap-3">
        <TabsList className="flex-wrap">
          {endpoints.map((endpoint) => (
            <TabsTrigger key={endpoint.id} value={endpoint.id} className="text-xs">
              {endpoint.label}
            </TabsTrigger>
          ))}
        </TabsList>

        {endpoints.map((endpoint) => {
          const rule = ruleSummary(endpoint);
          return (
            <TabsContent key={endpoint.id} value={endpoint.id} className="flex flex-col gap-2">
              <div className="flex flex-wrap items-center gap-2">
                <span className={cn("font-mono text-2xs font-semibold", METHOD_CLASS[endpoint.method])}>
                  {endpoint.method}
                </span>
                <code className="min-w-0 truncate font-mono text-xs text-foreground">{endpoint.path}</code>
                <Badge
                  variant="outline"
                  className={cn(
                    "max-w-64 truncate font-mono font-normal",
                    rule.tone === "locked" && "text-warning",
                    rule.tone === "open" && "text-success",
                  )}
                  title={rule.text}
                >
                  {rule.text}
                </Badge>
              </div>
              <Snippet code={endpoint.snippet} />
            </TabsContent>
          );
        })}
      </Tabs>

      <p className="text-2xs leading-snug text-muted-foreground">
        Snippets assume <code className="font-mono">const pb = new PocketBase("{window.location.origin}")</code> from
        the <code className="font-mono">pocketbase</code> npm package.
      </p>
    </section>
  );
}
