import { useEffect, useRef, useState } from "react";
import { Search, X } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * Full-text search box for a collection's records table — only rendered
 * when the collection has at least one `searchable` field (see
 * `routes/collection.tsx`). Applies on Enter/blur, not per keystroke,
 * same convention as `FilterBar`'s own filter expression input (a
 * request per keystroke against a possibly large table is the thing this
 * avoids).
 */
export function SearchBox({
  value,
  onApply,
  className,
}: {
  value: string;
  onApply: (next: string) => void;
  className?: string;
}) {
  const [draft, setDraft] = useState(value);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => setDraft(value), [value]);
  const dirty = draft !== value;

  const apply = (next: string) => {
    if (next !== value) onApply(next);
  };

  return (
    <div
      className={cn(
        "flex h-control-sm min-w-0 items-center gap-1.5 rounded-md border border-input bg-background px-2 transition-colors focus-within:border-ring",
        className,
      )}
    >
      <Search className="size-3.5 shrink-0 text-muted-foreground" />
      <input
        ref={inputRef}
        value={draft}
        spellCheck={false}
        autoComplete="off"
        autoCorrect="off"
        aria-label="Search records"
        placeholder="Search…"
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            apply(draft);
          }
          if (event.key === "Escape") {
            event.preventDefault();
            if (dirty) setDraft(value);
            else inputRef.current?.blur();
          }
        }}
        onBlur={() => apply(draft)}
        className="min-w-0 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground/70"
      />
      {draft.length > 0 ? (
        <button
          type="button"
          aria-label="Clear search"
          onClick={() => {
            setDraft("");
            apply("");
            inputRef.current?.focus();
          }}
          className="flex size-4 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <X className="size-3" />
        </button>
      ) : null}
    </div>
  );
}
