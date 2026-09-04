import { Check, Copy, KeyRound, Wrench } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { cb, describeFailure } from "@/lib/api";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";

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

  const { data: tools, isLoading, error } = useQuery({
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

  return (
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-4 p-page">
      <div className="flex flex-col gap-2">
        <h2 className="text-sm font-medium">MCP server</h2>
        <p className="max-w-measure text-sm text-muted-foreground">
          Every non-system collection is exposed to{" "}
          <a href="https://modelcontextprotocol.io" target="_blank" rel="noreferrer" className="underline underline-offset-2">
            Model Context Protocol
          </a>{" "}
          clients over JSON-RPC, rule-gated exactly like the REST API — an agent authenticates as an ordinary caller
          and gets exactly the access its token allows, nothing more.
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Endpoint</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <Snippet code={endpoint} />
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <KeyRound className="size-3.5 shrink-0" />
            Connecting requires a bearer token.
            <Button variant="link" size="sm" className="h-auto p-0" asChild>
              <Link to="/settings/api-keys">Create an API key for this →</Link>
            </Button>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Client config</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <p className="text-sm text-muted-foreground">
            Paste into Claude Desktop's <code className="font-mono">mcp_config.json</code> (or any client using the
            same shape), replacing the placeholder with a real key from the API keys page.
          </p>
          <Snippet code={configSnippet} />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Exposed tools</CardTitle>
        </CardHeader>
        <CardContent>
          {error ? (
            <Empty>
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Wrench />
                </EmptyMedia>
                <EmptyTitle>Couldn't load the tool list</EmptyTitle>
                <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : isLoading ? (
            <div className="flex flex-col gap-2">
              {Array.from({ length: 3 }, (_, i) => (
                <Skeleton key={i} className="h-row w-full" />
              ))}
            </div>
          ) : tools && tools.length > 0 ? (
            <ul className="flex flex-col divide-y divide-border">
              {tools.map((tool) => (
                <li key={tool.name} className="flex items-start justify-between gap-4 py-2.5">
                  <div className="flex flex-col gap-0.5">
                    <span className="font-mono text-xs font-medium">{tool.name}</span>
                    <span className="text-xs text-muted-foreground">{tool.description}</span>
                  </div>
                  <Badge variant="outline" className="shrink-0 font-normal text-muted-foreground">
                    {tool.name.split("_")[0]}
                  </Badge>
                </li>
              ))}
            </ul>
          ) : (
            <Empty>
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Wrench />
                </EmptyMedia>
                <EmptyTitle>No tools exposed yet</EmptyTitle>
                <EmptyDescription>
                  Every non-system collection becomes a set of tools automatically — create one to see it here.
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
