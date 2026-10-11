import { Check, Copy } from "lucide-react";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** Token pattern for pretty-printed JSON: a key, a string, or a bare literal. */
const TOKEN = /("(?:\\.|[^"\\])*")(\s*:)?|\b(true|false|null)\b|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/g;

/** Wrap keys, strings, numbers and literals in muted colours; everything else
 * (braces, commas, whitespace) stays as plain text. */
function highlight(json: string): React.ReactNode[] {
  const out: React.ReactNode[] = [];
  let last = 0;
  let n = 0;
  for (const m of json.matchAll(TOKEN)) {
    const at = m.index ?? 0;
    if (at > last) out.push(json.slice(last, at));
    if (m[1] !== undefined && m[2] !== undefined) {
      out.push(
        <span key={n++} className="text-foreground">
          {m[1]}
        </span>,
        m[2],
      );
    } else if (m[1] !== undefined) {
      out.push(
        <span key={n++} className="text-success">
          {m[1]}
        </span>,
      );
    } else if (m[3] !== undefined) {
      out.push(
        <span key={n++} className="text-info">
          {m[3]}
        </span>,
      );
    } else {
      out.push(
        <span key={n++} className="text-warning">
          {m[4]}
        </span>,
      );
    }
    last = at + m[0].length;
  }
  if (last < json.length) out.push(json.slice(last));
  return out;
}

/** A titled, copyable code block. Pass `json` (already pretty-printed) to get
 * light syntax colouring; pass `plain` for non-JSON text such as a mail body. */
export function CodeBlock({
  title,
  json,
  plain,
  copyLabel = "Copy",
  className,
}: {
  title?: string;
  json?: string;
  plain?: string;
  copyLabel?: string;
  className?: string;
}) {
  const { copy, status } = useCopyToClipboard();
  const text = json ?? plain ?? "";
  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      <div className="flex items-center justify-between gap-2">
        {title ? <h3 className="text-sm text-muted-foreground">{title}</h3> : <span />}
        <Button variant="ghost" size="xs" onClick={() => void copy(text)}>
          {status === "copied" ? <Check className="text-success" /> : <Copy />}
          {status === "copied" ? "Copied" : copyLabel}
        </Button>
      </div>
      <pre className="max-h-96 overflow-auto rounded-lg border border-border bg-surface-sunken px-3 py-2.5 font-mono text-xs leading-relaxed break-words whitespace-pre-wrap">
        {json !== undefined ? highlight(json) : text}
      </pre>
    </div>
  );
}
