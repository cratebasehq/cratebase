import { useState } from "react";
import { Braces } from "lucide-react";
import { mergeAttributes, type ChainedCommands } from "@tiptap/core";
import { EmailNode } from "@react-email/editor/core";
import { NodeViewWrapper, ReactNodeViewRenderer, type NodeViewProps } from "@tiptap/react";
import type { SlashCommandItem } from "@react-email/editor/ui";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";

declare module "@tiptap/core" {
  interface Commands<ReturnType> {
    variable: {
      /** Inserts a `variable` atom node at the cursor. */
      insertVariable: (path: string) => ReturnType;
    };
  }
}

/**
 * The keys "Insert variable" (both the sidebar list and the
 * `/variable` slash command) offer — the flattened sample-data JSON
 * plus `appName`/`appUrl`, kept live via a plain mutable ref rather
 * than an extension option so editing the sample data doesn't need to
 * reconfigure/remount the whole `Variable` extension (and every
 * `EditorProvider` under it).
 */
export const variableKeysRef: { current: string[] } = { current: [] };

/** The editor-only chip a `variable` node renders as — click it to
 * change which `{{path}}` it holds via the same key list the sidebar's
 * "Insert variable" button offers. Purely cosmetic: what actually gets
 * sent is [`Variable`]'s `renderToReactEmail`, the literal text
 * `{{path}}`. */
function VariableChip({ node, updateAttributes, selected }: NodeViewProps) {
  const path = typeof node.attrs.path === "string" ? node.attrs.path : "";
  const [open, setOpen] = useState(false);

  return (
    <NodeViewWrapper as="span" className="inline">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <span
            data-variable={path}
            contentEditable={false}
            className={`mx-0.5 inline-flex cursor-pointer items-center gap-1 rounded-full border px-2 py-0.5 align-baseline font-mono text-xs leading-none ${
              selected
                ? "border-primary bg-primary/10 text-primary"
                : "border-dashed border-border bg-muted text-muted-foreground hover:border-foreground/40"
            }`}
          >
            <Braces className="size-3" />
            {path || "path"}
          </span>
        </PopoverTrigger>
        <PopoverContent className="w-56 p-0" align="start">
          <Command>
            <CommandInput placeholder="Search variables…" />
            <CommandList>
              <CommandEmpty>No variables in the sample data yet.</CommandEmpty>
              <CommandGroup>
                {variableKeysRef.current.map((key) => (
                  <CommandItem
                    key={key}
                    value={key}
                    onSelect={() => {
                      updateAttributes({ path: key });
                      setOpen(false);
                    }}
                  >
                    <code className="font-mono text-xs">{`{{${key}}}`}</code>
                  </CommandItem>
                ))}
              </CommandGroup>
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
    </NodeViewWrapper>
  );
}

/**
 * A `{{path}}` placeholder, rendered as a small chip while editing and
 * as the literal mustache token on export — the custom extension the
 * brief asks for, built the way
 * [react.email's Custom Extensions docs](https://react.email/docs/editor/advanced/custom-extensions)
 * describe: an inline atom [`EmailNode`] whose `renderToReactEmail`
 * differs entirely from its editor-only `renderHTML`.
 *
 * A link/button's `href` is a plain text field (native to the
 * package's own link-editing UI), not rich content — a chip can't live
 * there, so typing `{{path}}` directly into that field is the
 * supported way to parameterize a URL. `render_mustache`
 * (`crates/mailer/src/mustache.rs`) doesn't care whether a `{{path}}`
 * came from a chip's export or from typed text; both are the same
 * literal token by the time the server sees it.
 */
export const Variable = EmailNode.create({
  name: "variable",
  group: "inline",
  inline: true,
  atom: true,
  selectable: true,

  addAttributes() {
    return {
      path: {
        default: "",
        parseHTML: (element: HTMLElement) => element.getAttribute("data-variable") ?? "",
        renderHTML: (attributes: { path: string }) => ({ "data-variable": attributes.path }),
      },
    };
  },

  parseHTML() {
    return [{ tag: "span[data-variable]" }];
  },

  renderHTML({ HTMLAttributes, node }) {
    const path = typeof node.attrs?.path === "string" ? node.attrs.path : "";
    return [
      "span",
      mergeAttributes(HTMLAttributes, { class: "cb-variable-chip" }),
      `{{${path}}}`,
    ];
  },

  addNodeView() {
    return ReactNodeViewRenderer(VariableChip);
  },

  addCommands() {
    return {
      insertVariable:
        (path: string) =>
        ({ chain }: { chain: () => ChainedCommands }) =>
          chain().insertContent({ type: "variable", attrs: { path } }).run(),
    };
  },

  // The only rendering path that matters for a sent email: a plain
  // string is a valid ReactNode and composes as a literal text node —
  // exactly the `{{path}}` the `{{var}}` engine resolves at send time.
  renderToReactEmail({ node }) {
    const path = typeof node.attrs?.path === "string" ? node.attrs.path : "";
    return `{{${path}}}`;
  },
});

/** `/variable` — inserts a chip defaulting to the first known key (or
 * the literal placeholder `path` if none is known yet, e.g. the sample
 * data is empty); click the inserted chip to change it. */
export const variableSlashCommandItem: SlashCommandItem = {
  title: "Variable",
  description: "Insert a {{path}} placeholder",
  icon: <Braces className="size-4" />,
  category: "Insert",
  searchTerms: ["variable", "placeholder", "mustache", "token", "merge field"],
  command: ({ editor, range }) => {
    const path = variableKeysRef.current[0] ?? "path";
    editor.chain().focus().deleteRange(range).run();
    editor.commands.insertVariable(path);
  },
};
