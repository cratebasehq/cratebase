import { useMemo, useState } from "react";
import { Check, Copy, KeyRound, Wrench } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { cb } from "@/lib/api";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { SETTINGS_GROUPS } from "@/lib/settings-nav";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { DataTableToolbar } from "@/components/layout/data-table-toolbar";
import { EmptyState, ErrorState, ListSkeleton } from "@/components/layout/states";
import { StatCard, StatusBadge } from "@/components/layout/stat-card";
import { SettingsPage, SettingsSection } from "@/components/settings/settings-form";

const ITEM = SETTINGS_GROUPS.flatMap((g) => g.tabs).find((t) => t.legacyPath === "/settings/mcp")!;

/** One entry of the MCP server's `tools/list` response
 * (`crates/server/src/mcp.rs`'s `build_tools`) — a `list_`/`get_`/
 * `create_`/`update_`/`delete_` operation over one non-system
 * collection. */
interface McpTool {
  name: string;
  description: string;
}

/** JSON-RPC envelope `POST /api/mcp` answers with — see `crate::mcp`'s
 * module doc for why this is hand-rolled JSON-RPC rather than SSE. */
interface McpRpcResponse<T> {
  result?: T;
  error?: { code: number; message: string };
}

const OPERATIONS = ["list", "get", "create", "update", "delete"] as const;
type Operation = (typeof OPERATIONS)[number];

/** `create_posts` → `{ operation: "create", collection: "posts" }`. Anything
 * that doesn't follow the verb_collection shape is kept whole under "other". */
function splitTool(tool: McpTool): { operation: Operation | null; collection: string } {
  const [verb, ...rest] = tool.name.split("_");
  if (rest.length > 0 && (OPERATIONS as readonly string[]).includes(verb)) {
    return { operation: verb as Operation, collection: rest.join("_") };
  }
  return { operation: null, collection: tool.name };
}

/** Write operations get the warning tone so it is obvious what an agent
 * holding this key could change. */
const OPERATION_TONE: Record<Operation, "info" | "warning" | "danger"> = {
  list: "info",
  get: "info",
  create: "warning",
  update: "warning",
  delete: "danger",
};

function Snippet({ code, label }: { code: string; label: string }) {
  const { copy, status } = useCopyToClipboard();
  return (
    <div className="relative">
      <pre
        tabIndex={0}
        aria-label={label}
        className="max-h-72 overflow-auto rounded-lg border border-border bg-surface-sunken py-2.5 pr-12 pl-3 font-mono text-xs leading-relaxed text-foreground"
      >
        {code}
      </pre>
      <Button
        type="button"
        variant="outline"
        size="icon-xs"
        onClick={() => void copy(code)}
        aria-label={status === "copied" ? "Copied" : `Copy ${label}`}
        className="absolute top-1.5 right-1.5"
      >
        {status === "copied" ? <Check /> : <Copy />}
      </Button>
    </div>
  );
}

/**
 * The already-shipped MCP server (`crates/server/src/mcp.rs`) exposes
 * every non-system collection as a `list_`/`get_`/`create_`/`update_`/
 * `delete_` tool, authenticated exactly like any other request —
 * including by an `_api_keys` bearer token, which is what makes it
 * usable by an external MCP client (Claude Desktop, an agent harness)
 * rather than only a browser session. This page shows the endpoint, a
 * ready-to-paste client config, and the live tool list so an operator
 * never has to guess what's exposed.
 */
export function McpPage() {
  const endpoint = cb.buildURL("/api/mcp");
  const [query, setQuery] = useState("");

  const {
    data: tools,
    isPending,
    isError,
    error,
    refetch,
  } = useQuery({
    queryKey: ["mcp-tools"],
    queryFn: async () => {
      const res = await cb.send<McpRpcResponse<{ tools: McpTool[] }>>("/api/mcp", {
        method: "POST",
        body: { jsonrpc: "2.0", id: 1, method: "tools/list", params: {} },
      });
      if (res.error) throw new Error(res.error.message);
      return res.result?.tools ?? [];
    },
  });

  const { groups, utilities } = useMemo(() => {
    const byCollection = new Map<string, { operation: Operation; tool: McpTool }[]>();
    const other: McpTool[] = [];
    for (const tool of tools ?? []) {
      const { operation, collection } = splitTool(tool);
      if (operation === null) other.push(tool);
      else byCollection.set(collection, [...(byCollection.get(collection) ?? []), { operation, tool }]);
    }
    return {
      groups: [...byCollection.entries()]
        .map(([collection, entries]) => ({
          collection,
          entries: entries.sort((a, b) => OPERATIONS.indexOf(a.operation) - OPERATIONS.indexOf(b.operation)),
        }))
        .sort((a, b) => a.collection.localeCompare(b.collection)),
      utilities: other.sort((a, b) => a.name.localeCompare(b.name)),
    };
  }, [tools]);

  const needle = query.trim().toLowerCase();
  const visible = groups.filter((g) => g.collection.toLowerCase().includes(needle));
  const visibleUtilities = utilities.filter((t) => t.name.toLowerCase().includes(needle));
  const writeTools = (tools ?? []).filter((t) => {
    const op = splitTool(t).operation;
    return op === "create" || op === "update" || op === "delete";
  }).length;

  const configSnippet = JSON.stringify(
    {
      mcpServers: {
        cratebase: {
          url: endpoint,
          headers: { Authorization: "Bearer <your-api-key>" },
        },
      },
    },
    null,
    2,
  );

  const curlSnippet = `curl -s ${endpoint} \\
  -H "Authorization: Bearer <your-api-key>" \\
  -H "Content-Type: application/json" \\
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'`;

  return (
    <SettingsPage
      title={ITEM.label}
      description={ITEM.description}
      width="form"
      action={
        <Button variant="outline" size="sm" asChild>
          <Link to="/settings/auth" search={{ tab: "api-keys" }}>
            <KeyRound />
            Create an API key
          </Link>
        </Button>
      }
    >
      <div className="grid grid-cols-3 gap-2">
        <StatCard
          index={0}
          label="Status"
          value={isError ? "Unreachable" : "Ready"}
          loading={isPending}
          tone={isError ? "danger" : "success"}
          hint="POST /api/mcp"
        />
        <StatCard index={1} label="Collections exposed" value={groups.length} loading={isPending} />
        <StatCard
          index={2}
          label="Tools"
          value={tools?.length ?? 0}
          loading={isPending}
          hint={tools ? `${writeTools} can change data` : undefined}
        />
      </div>

      <SettingsSection
        title="Connect a client"
        description="Any MCP client that speaks JSON-RPC over HTTP. Authenticate with an API key as a bearer token — the same permissions as the key apply."
      >
        <div className="flex flex-col gap-1.5">
          <p className="text-xs text-muted-foreground">Endpoint</p>
          <Snippet code={endpoint} label="MCP endpoint" />
        </div>
        <Tabs defaultValue="config">
          <TabsList>
            <TabsTrigger value="config">Client config</TabsTrigger>
            <TabsTrigger value="curl">Test with curl</TabsTrigger>
          </TabsList>
          <TabsContent value="config" className="flex flex-col gap-2 pt-1">
            <p className="text-xs text-muted-foreground">
              Paste into Claude Desktop's <code className="font-mono">mcp_config.json</code> (or any client using the same
              shape) and replace the placeholder with a real key.
            </p>
            <Snippet code={configSnippet} label="client config" />
          </TabsContent>
          <TabsContent value="curl" className="flex flex-col gap-2 pt-1">
            <p className="text-xs text-muted-foreground">
              Lists the tools your key can see. A <code className="font-mono">401</code> means the key is missing or wrong.
            </p>
            <Snippet code={curlSnippet} label="curl command" />
          </TabsContent>
        </Tabs>
      </SettingsSection>

      <section className="flex min-w-0 flex-col gap-2">
        <div className="flex flex-col">
          <h2 className="text-sm font-medium">Exposed tools</h2>
          <p className="text-xs text-muted-foreground">
            Every non-system collection becomes list, get, create, update and delete tools automatically.
          </p>
        </div>

        {isPending ? (
          <ListSkeleton rows={4} />
        ) : isError ? (
          <ErrorState title="Couldn't load the tool list" error={error} onRetry={() => void refetch()} compact />
        ) : groups.length === 0 && utilities.length === 0 ? (
          <EmptyState
            compact
            icon={<Wrench />}
            title="No tools exposed yet"
            description="Create a collection and its tools show up here on their own — nothing to enable."
            action={
              <Button size="sm" asChild>
                <Link to="/">Go to collections</Link>
              </Button>
            }
          />
        ) : (
          <>
            <DataTableToolbar
              search={{ value: query, onChange: setQuery, placeholder: "Filter collections and tools…" }}
            />
            {visible.length === 0 && visibleUtilities.length === 0 ? (
              <EmptyState compact title="Nothing matches" description={`No collection or tool is called "${query.trim()}".`} />
            ) : visible.length === 0 ? null : (
              <ul className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
                {visible.map((group) => (
                  <li key={group.collection} className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1.5 px-3 py-2">
                    <span className="font-mono text-xs font-medium">{group.collection}</span>
                    <ul className="flex flex-wrap gap-1">
                      {group.entries.map(({ operation, tool }) => (
                        <li key={tool.name}>
                          <span title={tool.description} aria-label={`${tool.name}: ${tool.description}`}>
                            <StatusBadge
                              tone={operation ? OPERATION_TONE[operation] : "neutral"}
                              className={cn("font-mono", operation === null && "font-normal")}
                            >
                              {operation ?? tool.name}
                            </StatusBadge>
                          </span>
                        </li>
                      ))}
                    </ul>
                  </li>
                ))}
              </ul>
            )}
            {visibleUtilities.length > 0 ? (
              <div className="flex flex-col gap-2 pt-2">
                <h3 className="text-sm font-medium">Utility tools</h3>
                <ul className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
                  {visibleUtilities.map((tool) => (
                    <li key={tool.name} className="flex flex-col gap-0.5 px-3 py-2">
                      <span className="font-mono text-xs font-medium">{tool.name}</span>
                      <span className="text-xs text-pretty text-muted-foreground">{tool.description}</span>
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
            <p className="text-xs text-muted-foreground">
              Hover an operation for the description the client sees. Blue tools read data; yellow ones create or
              change it; red ones delete it.
            </p>
          </>
        )}
      </section>
    </SettingsPage>
  );
}
