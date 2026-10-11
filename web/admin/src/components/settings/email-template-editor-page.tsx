import { useCallback, useEffect, useMemo, useRef, useState, type MutableRefObject } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useBlocker, useNavigate } from "@tanstack/react-router";
import { useTheme } from "next-themes";
import {
  ArrowLeft,
  Braces,
  Laptop,
  Moon,
  Palette,
  RotateCcw,
  RotateCw,
  Save,
  Send,
  Smartphone,
  Sun,
} from "lucide-react";
import { toast } from "sonner";
import CodeMirror, { type ReactCodeMirrorRef } from "@uiw/react-codemirror";
import { html as htmlLang } from "@codemirror/lang-html";
import type { Editor, JSONContent } from "@tiptap/core";
import { EditorProvider, useCurrentEditor } from "@tiptap/react";
import { StarterKit } from "@react-email/editor/extensions";
import { composeReactEmail } from "@react-email/editor/core";
import { BubbleMenu, SlashCommand, defaultSlashCommands } from "@react-email/editor/ui";
import { EmailTheming, imageSlashCommand, useEditorImage } from "@react-email/editor/plugins";
import "@react-email/editor/themes/default.css";
import "@react-email/editor/styles/bubble-menu.css";
import "@react-email/editor/styles/slash-command.css";
import { cb, currentSuperuser, describeFailure } from "@/lib/api";
import { cn } from "@/lib/utils";
import { useDevMailInboxAvailable, useSettings } from "@/hooks/use-settings";
import { useSaveShortcut } from "@/components/settings/use-save-shortcut";
import { settingsEmailTemplateEditorRoute } from "@/routes/settings-email-template-editor";
import { RuleField } from "@/components/collections/rule-field";
import { PageShell } from "@/components/layout/page";
import { StatusBadge, StatusDot } from "@/components/layout/stat-card";
import { EmptyState, ErrorState } from "@/components/layout/states";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { uploadEmailImage } from "@/components/settings/email-editor/image-upload";
import {
  Variable,
  variableDescriptionsRef,
  variableKeysRef,
  variableSlashCommandItem,
} from "@/components/settings/email-editor/variable-extension";
import {
  ThemePanel,
  buildEditorTheme,
  defaultEmailTheme,
  parseEmailTheme,
  type EmailThemeState,
} from "@/components/settings/email-editor/theme-panel";
import { StarterGalleryDialog } from "@/components/settings/email-editor/starter-gallery-dialog";
import type { StarterTemplate } from "@/components/settings/email-editor/starter-templates";

/** The opaque `_emailTemplates.design` shape this editor writes —
 * the editor's own tiptap document plus the Theme tab's state, kept as
 * a sibling rather than embedded some other way since
 * `@react-email/editor` has no built-in "persist my theme choice with
 * the document" story we've wired up. A row saved before this shipped
 * (or one whose `design` was never anything but raw HTML) has neither
 * — see `loadDesign` below. */
interface EmailDesign {
  tiptap: JSONContent;
  theme: EmailThemeState;
}

interface TemplateRecord {
  id: string;
  key: string;
  name: string;
  subject: string;
  html: string;
  text: string;
  locale: string;
  layout: boolean;
  description: string;
  sendRule: string | null;
  editor: "visual" | "html" | "";
  design: unknown;
}

interface PreviewResult {
  subject: string;
  html: string;
  text?: string;
}

/** Everything the form edits, captured at one moment — the "saved"
 * baseline the dirty indicator and Reset compare against. */
interface Snapshot {
  key: string;
  name: string;
  subject: string;
  locale: string;
  layout: boolean;
  html: string;
  text: string;
  sendRule: string | null;
  mode: "visual" | "html";
  theme: EmailThemeState;
  tiptap: JSONContent | null;
}

/** HTML only counts while the HTML editor is the source of truth —
 * in Visual mode `html` is a stale cache of whatever was last composed. */
function fingerprint(s: Snapshot): string {
  return JSON.stringify([
    s.key,
    s.name,
    s.subject,
    s.locale,
    s.layout,
    s.mode === "html" ? s.html : "",
    s.text,
    s.sendRule,
    s.mode,
    s.theme,
  ]);
}

interface VariableEntry {
  key: string;
  description: string;
}

/** A crude, dependency-free HTML→text approximation for the "auto-derive"
 * button — mirrors `crates/mailer/src/mustache.rs`'s `html_to_text`
 * closely enough to be a useful starting point, not a byte-for-byte port
 * (the server's own version is what actually runs if `text` is left
 * blank at send time; this button is purely a client-side convenience
 * for an author who wants to start from something and edit it). */
function htmlToTextApprox(html: string): string {
  const withBreaks = html
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/(p|div|tr|h[1-6])>/gi, "\n")
    .replace(/<[^>]+>/g, "");
  const decoded = withBreaks
    .replace(/&nbsp;/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'");
  return decoded
    .split("\n")
    .map((line) => line.trim())
    .filter((line, i, arr) => line !== "" || arr[i - 1] !== "")
    .join("\n")
    .trim();
}

/** Flattens a JSON value into dotted paths (with a short sample of the
 * leaf value) for the "Insert variable" menu — `{ user: { name: "Ada" } }`
 * becomes `[{ key: "user.name", sample: "Ada" }]`. Arrays are skipped
 * (a `{{var}}` path has no index syntax). */
function flattenEntries(value: unknown, prefix = ""): { key: string; sample: string }[] {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return prefix ? [{ key: prefix, sample: sampleLabel(value) }] : [];
  }
  const out: { key: string; sample: string }[] = [];
  for (const [key, v] of Object.entries(value as Record<string, unknown>)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (v !== null && typeof v === "object" && !Array.isArray(v)) {
      out.push(...flattenEntries(v, path));
    } else {
      out.push({ key: path, sample: sampleLabel(v) });
    }
  }
  return out;
}

function sampleLabel(value: unknown): string {
  if (value === null || value === undefined) return "empty";
  const text = typeof value === "string" ? `"${value}"` : Array.isArray(value) ? "[array]" : String(value);
  return text.length > 36 ? `${text.slice(0, 35)}…` : text;
}

function insertAtCursor(view: ReactCodeMirrorRef["view"], text: string) {
  if (!view) return;
  const { from, to } = view.state.selection.main;
  view.dispatch({ changes: { from, to, insert: text }, selection: { anchor: from + text.length } });
  view.focus();
}

/** Parses `_emailTemplates.design` into the visual editor's tiptap
 * document plus theme state — tolerant of a row saved before the
 * `{ tiptap, theme }` shape existed (`design` was the raw tiptap
 * document itself back then), or one that's never been opened in the
 * visual editor at all (`null`). */
function loadDesign(
  design: unknown,
  brandColor: string | undefined,
): { tiptap: JSONContent | null; theme: EmailThemeState } {
  if (design && typeof design === "object") {
    const obj = design as Record<string, unknown>;
    if ("tiptap" in obj && obj.tiptap && typeof obj.tiptap === "object") {
      return {
        tiptap: obj.tiptap as JSONContent,
        theme: parseEmailTheme(obj.theme, brandColor),
      };
    }
    if ("type" in obj) {
      // A pre-theme-tab row: `design` was the raw tiptap document.
      return { tiptap: design as JSONContent, theme: defaultEmailTheme(brandColor) };
    }
  }
  return { tiptap: null, theme: defaultEmailTheme(brandColor) };
}

const SAMPLE_DATA_PLACEHOLDER = `{\n  "user": { "name": "Ada" }\n}`;

const BUILTIN_VARIABLES: VariableEntry[] = [
  { key: "appName", description: "This instance's name, from Application settings." },
  { key: "appUrl", description: "This instance's public URL, from Application settings." },
];

const EMAIL_PATTERN = /^\S+@\S+\.\S+$/;

const MOD_KEY =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘" : "Ctrl+";

/** Lets a preview iframe honour the *emulated* colour scheme: the iframe
 * follows the OS preference, not the admin theme, so the author's
 * `@media (prefers-color-scheme: …)` blocks are rewritten to always / never
 * match. The HTML that is saved or sent is untouched. */
function emulateColorScheme(html: string, scheme: "light" | "dark"): string {
  return html.replace(/\(\s*prefers-color-scheme\s*:\s*(dark|light)\s*\)/gi, (_, wanted: string) =>
    wanted.toLowerCase() === scheme ? "(min-width: 0px)" : "(min-width: 100000px)",
  );
}

/** A stable reference to whatever `uploadImage` a render currently
 * closes over, so `useEditorImage` (called once) always calls the
 * latest one without needing the extensions array — and therefore the
 * editor itself — to be rebuilt every time `appUrl` changes. */
function useLatestUploadImage(appUrl: string) {
  const ref = useRef(appUrl);
  ref.current = appUrl;
  return useMemo(
    () => (file: File) => uploadEmailImage(file, ref.current).catch((error: unknown) => {
      toast.error("Image upload failed", {
        description: error instanceof Error ? error.message : String(error),
      });
      throw error;
    }),
    [],
  );
}

/** Captures the live `Editor` instance into `ref` — `EditorProvider`
 * has no `ref` prop of its own (unlike the old top-level `EmailEditor`
 * component), so this tiny child (rendered inside the provider, with
 * access to `useCurrentEditor`) is how the page keeps one. */
function EditorInstanceCapture({
  editorRef,
  onUpdate,
}: {
  editorRef: MutableRefObject<Editor | null>;
  onUpdate: () => void;
}) {
  const { editor } = useCurrentEditor();
  useEffect(() => {
    editorRef.current = editor;
    if (!editor) return;
    const handler = () => onUpdate();
    editor.on("update", handler);
    return () => {
      editor.off("update", handler);
    };
  }, [editor, editorRef, onUpdate]);
  return null;
}

const SLASH_COMMAND_ITEMS = [...defaultSlashCommands, imageSlashCommand, variableSlashCommandItem];

/** Searchable "Insert variable" popover — each entry says where its value
 * comes from, so authors don't have to guess what `{{appUrl}}` is. */
function VariableMenu({
  entries,
  onInsert,
  label = "Insert variable",
}: {
  entries: VariableEntry[];
  onInsert: (key: string) => void;
  label?: string;
}) {
  const [open, setOpen] = useState(false);
  // After picking, focus belongs to the editor the variable went into —
  // not back on this trigger.
  const picked = useRef(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button type="button" variant="ghost" size="xs">
          <Braces />
          {label}
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        className="w-80 p-0"
        onCloseAutoFocus={(event) => {
          if (picked.current) event.preventDefault();
          picked.current = false;
        }}
      >
        <Command>
          <CommandInput placeholder="Search variables…" aria-label="Search variables" />
          <CommandList>
            <CommandEmpty>No variables match.</CommandEmpty>
            <CommandGroup>
              {entries.map((entry) => (
                <CommandItem
                  key={entry.key}
                  value={`${entry.key} ${entry.description}`}
                  onSelect={() => {
                    picked.current = true;
                    onInsert(entry.key);
                    setOpen(false);
                  }}
                  className="flex-col items-start gap-0.5"
                >
                  <code className="font-mono text-xs">{`{{${entry.key}}}`}</code>
                  <span className="text-xs text-muted-foreground">{entry.description}</span>
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

function FieldError({ id, children }: { id: string; children: string }) {
  return (
    <p id={id} role="alert" className="text-xs text-destructive">
      {children}
    </p>
  );
}

function LoadingSkeleton() {
  return (
    <PageShell scroll={false} className="gap-0 p-0 sm:p-0">
      <div className="flex shrink-0 items-center gap-3 border-b border-border px-3 py-2 sm:px-page">
        <Skeleton className="size-control-sm" />
        <Skeleton className="h-5 w-48" />
        <div className="ml-auto flex gap-2">
          <Skeleton className="h-control-sm w-20" />
          <Skeleton className="h-control-sm w-24" />
          <Skeleton className="h-control-sm w-16" />
        </div>
      </div>
      <div className="grid min-h-0 flex-1 grid-cols-1 lg:grid-cols-[minmax(0,1fr)_28rem]">
        <div className="flex flex-col gap-4 p-3 sm:p-page">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            <Skeleton className="h-control-lg" />
            <Skeleton className="h-control-lg" />
            <Skeleton className="h-control-lg" />
          </div>
          <Skeleton className="h-control-lg" />
          <Skeleton className="h-72 w-full" />
        </div>
        <div className="hidden flex-col gap-3 border-l border-border p-page lg:flex">
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-[36rem] w-full" />
        </div>
      </div>
    </PageShell>
  );
}

/**
 * Create or edit one `_emailTemplates` row: subject/name/locale/layout,
 * the `sendRule` gate (`RuleField`, same component a collection API rule
 * uses), an HTML editor (CodeMirror) or the visual editor (built on
 * `@react-email/editor`'s lower-level `EditorProvider` composition, so
 * a custom `/variable` slash command and a live-editable theme can sit
 * alongside its full block set), a live preview rendered through
 * `POST /api/mails/preview` against editable sample data, and a test
 * send through `POST /api/mails/send`.
 */
export function EmailTemplateEditorPage() {
  const { id } = settingsEmailTemplateEditorRoute.useParams();
  const isNew = id === "new";
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const { resolvedTheme } = useTheme();
  const { data: devMailAvailable } = useDevMailInboxAvailable();
  const { data: settings } = useSettings();
  const appUrl = settings?.meta.appURL ?? "";

  const {
    data: existing,
    isLoading,
    isError,
    error: loadError,
    refetch,
  } = useQuery({
    queryKey: ["email-templates", id],
    queryFn: () => cb.collection("_emailTemplates").one(id) as unknown as Promise<TemplateRecord>,
    enabled: !isNew,
  });

  const [key, setKey] = useState("");
  const [name, setName] = useState("");
  const [subject, setSubject] = useState("");
  const [localeValue, setLocaleValue] = useState("");
  const [layout, setLayout] = useState(true);
  const [htmlBody, setHtmlBody] = useState("");
  const [text, setText] = useState("");
  const [sendRule, setSendRule] = useState<string | null>(null);
  const [editorMode, setEditorMode] = useState<"visual" | "html">("html");
  const [tiptapJson, setTiptapJson] = useState<JSONContent | null>(null);
  const [themeState, setThemeState] = useState<EmailThemeState>(defaultEmailTheme(undefined));
  const [loadedOnce, setLoadedOnce] = useState(false);
  const [starterPicked, setStarterPicked] = useState(!isNew);
  const [baseline, setBaseline] = useState<Snapshot | null>(null);
  // The visual document isn't React state, so "it changed" is tracked as a
  // flag set by the editor's update event (and by picking a starter).
  const [contentTouched, setContentTouched] = useState(false);
  const [resetNonce, setResetNonce] = useState(0);

  // Seed local form state once the record loads (or immediately for a
  // new template) — a query refetch afterwards must not clobber in-flight
  // edits.
  useEffect(() => {
    if (loadedOnce) return;
    if (isNew) {
      const theme = defaultEmailTheme(settings?.meta.brandColor);
      setThemeState(theme);
      setBaseline({
        key: "",
        name: "",
        subject: "",
        locale: "",
        layout: true,
        html: "",
        text: "",
        sendRule: null,
        mode: "html",
        theme,
        tiptap: null,
      });
      setLoadedOnce(true);
      return;
    }
    if (!existing) return;
    const mode = existing.editor === "visual" ? "visual" : "html";
    const { tiptap, theme } = loadDesign(existing.design, settings?.meta.brandColor);
    setKey(existing.key);
    setName(existing.name);
    setSubject(existing.subject);
    setLocaleValue(existing.locale);
    setLayout(existing.layout);
    setHtmlBody(existing.html);
    setText(existing.text);
    setSendRule(existing.sendRule);
    setEditorMode(mode);
    setTiptapJson(tiptap);
    setThemeState(theme);
    setBaseline({
      key: existing.key,
      name: existing.name,
      subject: existing.subject,
      locale: existing.locale,
      layout: existing.layout,
      html: existing.html,
      text: existing.text,
      sendRule: existing.sendRule,
      mode,
      theme,
      tiptap,
    });
    setLoadedOnce(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only re-seed once existing/settings first arrive
  }, [existing, isNew, loadedOnce, settings]);

  const codeMirrorRef = useRef<ReactCodeMirrorRef>(null);
  const editorInstanceRef = useRef<Editor | null>(null);
  const subjectRef = useRef<HTMLInputElement>(null);
  const keyRef = useRef<HTMLInputElement>(null);
  const [visualTick, setVisualTick] = useState(0);
  const handleEditorUpdate = useCallback(() => {
    setVisualTick((t) => t + 1);
    setContentTouched(true);
  }, []);

  const [sampleData, setSampleData] = useState(SAMPLE_DATA_PLACEHOLDER);
  const sampleParse = useMemo<{ value: Record<string, unknown>; error: string | null }>(() => {
    if (!sampleData.trim()) return { value: {}, error: null };
    try {
      const parsed: unknown = JSON.parse(sampleData);
      if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
        return { value: {}, error: "Sample data must be a JSON object, like { \"user\": { \"name\": \"Ada\" } }." };
      }
      return { value: parsed as Record<string, unknown>, error: null };
    } catch (failure) {
      return { value: {}, error: `Not valid JSON — ${failure instanceof Error ? failure.message : "parse error"}.` };
    }
  }, [sampleData]);
  const variableEntries = useMemo<VariableEntry[]>(() => {
    const fromSample = flattenEntries(sampleParse.value).map((e) => ({
      key: e.key,
      description: `From sample data · ${e.sample}`,
    }));
    const taken = new Set(fromSample.map((e) => e.key));
    return [...fromSample, ...BUILTIN_VARIABLES.filter((e) => !taken.has(e.key))];
  }, [sampleParse.value]);
  // The Variable extension's chip/slash-command UI reads these refs live
  // rather than an extension option, so editing the sample data never
  // needs to reconfigure (and therefore remount) the editor.
  variableKeysRef.current = variableEntries.map((e) => e.key);
  variableDescriptionsRef.current = Object.fromEntries(variableEntries.map((e) => [e.key, e.description]));

  const uploadImage = useLatestUploadImage(appUrl);
  const imageExtension = useEditorImage({ uploadImage });
  const editorTheme = useMemo(() => buildEditorTheme(themeState), [themeState]);
  const extensions = useMemo(
    () => [StarterKit, EmailTheming.configure({ theme: editorTheme }), imageExtension, Variable],
    [editorTheme, imageExtension],
  );
  // Changing the theme requires remounting `EditorProvider` (its own
  // documented "Dynamic Switching" pattern — the theme extension is
  // configured once, at construction) — capture whatever's currently in
  // the live document first so in-progress edits survive the remount.
  const editorRemountKey = `${JSON.stringify(themeState)}:${resetNonce}`;
  function updateTheme(next: EmailThemeState) {
    const latest = editorInstanceRef.current?.getJSON();
    if (latest) setTiptapJson(latest);
    setThemeState(next);
  }

  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [rendering, setRendering] = useState(false);
  const [previewNonce, setPreviewNonce] = useState(0);
  const [previewWidth, setPreviewWidth] = useState<"desktop" | "mobile">("desktop");
  const [previewScheme, setPreviewScheme] = useState<"light" | "dark">("light");
  const previewDoc = useMemo(
    () => (preview ? emulateColorScheme(preview.html, previewScheme) : ""),
    [preview, previewScheme],
  );

  const [sideTab, setSideTab] = useState("preview");
  // Below `lg` the editor and the side panel share one column.
  const [mobilePane, setMobilePane] = useState<"edit" | "panel">("edit");

  const [testTo, setTestTo] = useState(currentSuperuser()?.email ?? "");
  const [testOpen, setTestOpen] = useState(false);
  const testSend = useMutation({
    mutationFn: async () => {
      const { html: sendHtml, text: sendText } = await currentEmail();
      return cb.mails.send({
        to: testTo,
        subject,
        html: sendHtml,
        text: sendText || undefined,
        data: sampleParse.value,
        locale: localeValue || undefined,
      });
    },
    onSuccess: (result) => {
      if (result.status === "failed") {
        toast.error(`Test send to ${testTo} failed`, { description: result.error });
      } else {
        setTestOpen(false);
        toast.success(
          devMailAvailable ? `Test sent to ${testTo} — check the dev mail inbox` : `Test sent to ${testTo}`,
        );
      }
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  /** The HTML/text this template would send *right now*, including
   * unsaved visual-editor edits — used by both the preview and test send
   * so neither ever silently previews stale content. */
  async function currentEmail(): Promise<{ html: string; text: string }> {
    if (editorMode === "visual" && editorInstanceRef.current) {
      try {
        const composed = await composeReactEmail({ editor: editorInstanceRef.current });
        return { html: composed.html, text: composed.text };
      } catch {
        // Fall through to the last-synced HTML below.
      }
    }
    return { html: htmlBody, text };
  }

  // Debounced live preview: re-renders through the real server pipeline
  // (`POST /api/mails/preview`) whenever the subject, HTML/text, sample
  // data, or locale changes — including visual-editor keystrokes, via
  // `visualTick`.
  useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(() => {
      void (async () => {
        if (!cancelled) setRendering(true);
        const { html: previewHtml, text: previewText } = await currentEmail();
        try {
          const result = await cb.mails.preview({
            subject,
            html: previewHtml,
            text: previewText || undefined,
            data: sampleParse.value,
            locale: localeValue || undefined,
          });
          if (!cancelled) {
            setPreview(result);
            setPreviewError(null);
          }
        } catch (failure) {
          if (!cancelled) setPreviewError(describeFailure(failure).detail);
        } finally {
          if (!cancelled) setRendering(false);
        }
      })();
    }, 400);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- currentEmail reads live state via closures
  }, [subject, htmlBody, text, sampleParse.value, localeValue, visualTick, editorMode, previewNonce]);

  // --- validation -------------------------------------------------------
  const [attempted, setAttempted] = useState(false);
  const [touched, setTouched] = useState<{ key?: boolean; subject?: boolean }>({});
  const keyError = key.trim() ? null : "Key is required — triggers and API calls refer to the template by it.";
  const subjectError = subject.trim() ? null : "Subject is required — it's the line recipients see in their inbox.";
  const showKeyError = keyError !== null && (attempted || touched.key);
  const showSubjectError = subjectError !== null && (attempted || touched.subject);
  const testToValid = EMAIL_PATTERN.test(testTo.trim());

  // --- dirty tracking + leave guard --------------------------------------
  const current: Snapshot = {
    key,
    name,
    subject,
    locale: localeValue,
    layout,
    html: htmlBody,
    text,
    sendRule,
    mode: editorMode,
    theme: themeState,
    tiptap: null,
  };
  const dirty = baseline !== null && (contentTouched || fingerprint(current) !== fingerprint(baseline));
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
  // Set around the post-create redirect (same editor, new URL) so the
  // guard doesn't ask about edits that were just saved.
  const leavingRef = useRef(false);

  const blocker = useBlocker({
    shouldBlockFn: ({ current: from, next }) =>
      dirtyRef.current && !leavingRef.current && from.pathname !== next.pathname,
    enableBeforeUnload: false,
    withResolver: true,
  });

  const save = useMutation({
    mutationFn: async () => {
      let finalHtml = htmlBody;
      let finalText = text;
      let finalDesign: EmailDesign | null = tiptapJson ? { tiptap: tiptapJson, theme: themeState } : null;
      if (editorMode === "visual" && editorInstanceRef.current) {
        const composed = await composeReactEmail({ editor: editorInstanceRef.current });
        finalHtml = composed.html;
        finalText = composed.text || text;
        finalDesign = { tiptap: editorInstanceRef.current.getJSON(), theme: themeState };
      }
      const body: Record<string, unknown> = {
        key,
        name,
        subject,
        html: finalHtml,
        text: finalText,
        locale: localeValue,
        layout,
        sendRule,
        editor: editorMode,
        design: editorMode === "visual" ? finalDesign : null,
      };
      const saved = isNew
        ? await cb.collection("_emailTemplates").create(body)
        : await cb.collection("_emailTemplates").update(id, body);
      const snapshot: Snapshot = {
        key,
        name,
        subject,
        locale: localeValue,
        layout,
        html: finalHtml,
        text: finalText,
        sendRule,
        mode: editorMode,
        theme: themeState,
        tiptap: editorMode === "visual" ? (finalDesign?.tiptap ?? null) : null,
      };
      return { saved, snapshot };
    },
    onSuccess: ({ saved, snapshot }) => {
      toast.success(isNew ? `Template ${snapshot.key} created` : `Template ${snapshot.key} saved`);
      setBaseline(snapshot);
      setContentTouched(false);
      if (snapshot.mode === "visual") setText(snapshot.text);
      void queryClient.invalidateQueries({ queryKey: ["email-templates"] });
      const record = saved as unknown as TemplateRecord;
      queryClient.setQueryData(["email-templates", record.id], record);
      if (isNew) {
        // Stay in this editor, but on the record's own URL — otherwise
        // the next Save would create a second copy.
        leavingRef.current = true;
        void navigate({ to: "/settings/email-templates/$id", params: { id: record.id }, replace: true }).then(() => {
          leavingRef.current = false;
        });
      }
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  function attemptSave() {
    if (save.isPending) return;
    setAttempted(true);
    if (keyError || subjectError) {
      toast.error("Can't save yet", { description: keyError ?? subjectError ?? undefined });
      (keyError ? keyRef : subjectRef).current?.focus();
      return;
    }
    save.mutate();
  }
  useSaveShortcut({ dirty, canSave: true, onSave: attemptSave });

  const [confirmReset, setConfirmReset] = useState(false);
  function resetToBaseline() {
    if (!baseline) return;
    setKey(baseline.key);
    setName(baseline.name);
    setSubject(baseline.subject);
    setLocaleValue(baseline.locale);
    setLayout(baseline.layout);
    setHtmlBody(baseline.html);
    setText(baseline.text);
    setSendRule(baseline.sendRule);
    setEditorMode(baseline.mode);
    setThemeState(baseline.theme);
    setTiptapJson(baseline.tiptap);
    setResetNonce((n) => n + 1);
    setContentTouched(false);
    setAttempted(false);
    setTouched({});
    setConfirmReset(false);
    toast("Changes discarded");
  }

  function pickStarter(template: StarterTemplate) {
    setTiptapJson(template.content);
    if (template.subject) setSubject(template.subject);
    setEditorMode("visual");
    setContentTouched(true);
    setStarterPicked(true);
  }

  /** Re-parses content across the HTML/visual boundary, per
   * `templates.mdx`'s own documented "switching modes round-trips
   * through both, which can lose formatting" note: visual → HTML
   * captures the live document's exported HTML into `htmlBody`; HTML →
   * visual clears `tiptapJson` so the visual editor's `content` prop
   * falls back to (and parses) the current `htmlBody` string. Either
   * way the *other* mode always reflects the most recent edits, rather
   * than silently reverting to whatever was last synced. */
  async function switchEditorMode(next: "visual" | "html") {
    if (next === editorMode) return;
    if (editorMode === "visual" && editorInstanceRef.current) {
      try {
        const composed = await composeReactEmail({ editor: editorInstanceRef.current });
        setHtmlBody(composed.html);
      } catch (failure) {
        toast.error("Couldn't switch to the HTML editor", {
          description: failure instanceof Error ? failure.message : "Exporting the visual document failed.",
        });
        return;
      }
      setTiptapJson(null);
    } else if (editorMode === "html") {
      setTiptapJson(null);
    }
    setEditorMode(next);
  }

  function insertVariable(variable: string) {
    if (editorMode === "visual") editorInstanceRef.current?.commands.insertVariable(variable);
    else insertAtCursor(codeMirrorRef.current?.view, `{{${variable}}}`);
    setMobilePane("edit");
  }

  function insertIntoSubject(variable: string) {
    const token = `{{${variable}}}`;
    const el = subjectRef.current;
    const start = el?.selectionStart ?? subject.length;
    const end = el?.selectionEnd ?? start;
    setSubject(subject.slice(0, start) + token + subject.slice(end));
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(start + token.length, start + token.length);
    });
  }

  // Gate on `loadedOnce`, not just `isLoading`: react-query resolves
  // `isLoading` to `false` the instant `existing` arrives, one render
  // before the "seed local state" effect above has actually run — the
  // visual editor must never mount before `tiptapJson` reflects the
  // loaded row, since nothing here forces a remount just because state
  // changed (only a theme change does, via `editorRemountKey`).
  if (!isNew && isError && !loadedOnce) {
    return (
      <PageShell width="narrow">
        <Button variant="ghost" size="sm" className="w-fit" asChild>
          <Link to="/settings/email" search={{ tab: "templates" }}>
            <ArrowLeft />
            Email templates
          </Link>
        </Button>
        <ErrorState title="Couldn't load this template" error={loadError} onRetry={() => void refetch()} />
      </PageShell>
    );
  }
  if (!isNew && (isLoading || !loadedOnce)) return <LoadingSkeleton />;

  const title = name.trim() || key.trim() || "New template";

  const toolbarToggleItem = "px-2";

  return (
    <PageShell scroll={false} className="gap-0 p-0 sm:p-0">
      {isNew ? <StarterGalleryDialog open={!starterPicked} onPick={pickStarter} /> : null}

      <header className="z-sticky flex shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b border-border bg-background px-3 py-2 sm:px-page">
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <Button variant="ghost" size="icon-sm" asChild>
            <Link to="/settings/email" search={{ tab: "templates" }} aria-label="Back to email templates">
              <ArrowLeft />
            </Link>
          </Button>
          <h1 className="min-w-0 truncate text-base font-semibold tracking-tight">{title}</h1>
          {dirty ? (
            <StatusBadge tone="warning">Unsaved changes</StatusBadge>
          ) : (
            <StatusBadge tone="neutral">{isNew ? "Not created yet" : "All changes saved"}</StatusBadge>
          )}
        </div>

        <div className="flex shrink-0 flex-wrap items-center gap-2">
          <Button variant="ghost" size="sm" disabled={!dirty || save.isPending} onClick={() => setConfirmReset(true)}>
            <RotateCcw />
            Reset
          </Button>

          <Popover open={testOpen} onOpenChange={setTestOpen}>
            <PopoverTrigger asChild>
              <Button variant="outline" size="sm">
                <Send />
                Send test
              </Button>
            </PopoverTrigger>
            <PopoverContent align="end" className="w-80">
              <form
                className="flex flex-col gap-2"
                onSubmit={(event) => {
                  event.preventDefault();
                  if (!testSend.isPending && testToValid && !subjectError) testSend.mutate();
                }}
              >
                <Label htmlFor="template-test-to">Send a test to</Label>
                <div className="flex gap-1.5">
                  <Input
                    id="template-test-to"
                    type="email"
                    value={testTo}
                    onChange={(e) => setTestTo(e.target.value)}
                    placeholder="you@example.com"
                    autoComplete="email"
                    className="h-control-sm flex-1"
                    aria-invalid={testTo !== "" && !testToValid}
                    aria-describedby="template-test-help"
                  />
                  <Button
                    type="submit"
                    variant="outline"
                    size="sm"
                    disabled={testSend.isPending || !testToValid || subjectError !== null}
                  >
                    {testSend.isPending ? <Spinner /> : <Send />}
                    Send
                  </Button>
                </div>
                <p id="template-test-help" className="text-xs text-muted-foreground">
                  {subjectError
                    ? "Add a subject first — a test needs one."
                    : "Sends what's in the editor now, including unsaved changes, rendered with the sample data."}
                </p>
                {devMailAvailable ? (
                  <Link
                    to="/settings/email"
                    search={{ tab: "dev-inbox" }}
                    className="text-xs text-muted-foreground hover:text-foreground hover:underline"
                  >
                    No real SMTP configured — test sends land in the dev mail inbox →
                  </Link>
                ) : null}
              </form>
            </PopoverContent>
          </Popover>

          <Button
            size="sm"
            disabled={save.isPending || !dirty}
            onClick={attemptSave}
            title={`Save (${MOD_KEY}S)`}
            aria-keyshortcuts="Control+S Meta+S"
          >
            {save.isPending ? <Spinner /> : <Save />}
            {isNew ? "Create template" : "Save"}
          </Button>
        </div>
      </header>

      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border px-3 py-1.5 sm:px-page lg:hidden">
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          spacing={0}
          value={mobilePane}
          aria-label="Editor or preview"
          onValueChange={(next) => {
            if (next === "edit" || next === "panel") setMobilePane(next);
          }}
        >
          <ToggleGroupItem value="edit" className={toolbarToggleItem}>
            Editor
          </ToggleGroupItem>
          <ToggleGroupItem value="panel" className={toolbarToggleItem}>
            Preview &amp; settings
          </ToggleGroupItem>
        </ToggleGroup>
      </div>

      <div className="grid min-h-0 flex-1 grid-cols-1 lg:grid-cols-[minmax(0,1fr)_28rem] xl:grid-cols-[minmax(0,1fr)_34rem]">
        <div
          className={cn(
            "min-h-0 flex-col overflow-y-auto border-border lg:flex lg:border-r",
            mobilePane === "edit" ? "flex" : "hidden",
          )}
        >
          <div className="flex flex-col gap-5 p-3 sm:p-page">
            <section aria-label="Template details" className="flex flex-col gap-3">
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-[minmax(0,1.2fr)_minmax(0,1.2fr)_minmax(0,0.8fr)]">
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="template-key">Key</Label>
                  <Input
                    id="template-key"
                    ref={keyRef}
                    value={key}
                    onChange={(e) => setKey(e.target.value)}
                    onBlur={() => setTouched((t) => ({ ...t, key: true }))}
                    placeholder="welcome-email"
                    spellCheck={false}
                    className="font-mono"
                    aria-invalid={showKeyError}
                    aria-describedby={showKeyError ? "template-key-error" : "template-key-help"}
                  />
                  {showKeyError ? (
                    <FieldError id="template-key-error">{keyError}</FieldError>
                  ) : (
                    <p id="template-key-help" className="text-xs text-muted-foreground">
                      The identifier triggers and API calls use.
                    </p>
                  )}
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="template-name">Name</Label>
                  <Input
                    id="template-name"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder="Welcome email"
                  />
                  <p className="text-xs text-muted-foreground">A friendly label for the template list.</p>
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="template-locale">Locale</Label>
                  <Input
                    id="template-locale"
                    value={localeValue}
                    onChange={(e) => setLocaleValue(e.target.value)}
                    placeholder="default"
                    spellCheck={false}
                    className="font-mono"
                    aria-describedby="template-locale-help"
                  />
                  <p id="template-locale-help" className="text-xs text-muted-foreground">
                    Blank is the default; use a code like <code className="font-mono">fr</code> for a translation.
                  </p>
                </div>
              </div>

              <div className="flex flex-col gap-1.5">
                <div className="flex items-center justify-between gap-2">
                  <Label htmlFor="template-subject">Subject</Label>
                  <VariableMenu entries={variableEntries} onInsert={insertIntoSubject} />
                </div>
                <Input
                  id="template-subject"
                  ref={subjectRef}
                  value={subject}
                  onChange={(e) => setSubject(e.target.value)}
                  onBlur={() => setTouched((t) => ({ ...t, subject: true }))}
                  placeholder="Welcome to {{appName}}!"
                  className="font-mono"
                  aria-invalid={showSubjectError}
                  aria-describedby={showSubjectError ? "template-subject-error" : "template-subject-help"}
                />
                {showSubjectError ? (
                  <FieldError id="template-subject-error">{subjectError}</FieldError>
                ) : (
                  <p id="template-subject-help" className="text-xs text-muted-foreground">
                    Variables work here too — <code className="font-mono">{"{{user.name}}"}</code>.
                  </p>
                )}
              </div>
            </section>

            <section aria-label="Body" className="flex flex-col gap-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <Label id="template-editor-mode-label">Body</Label>
                  <ToggleGroup
                    type="single"
                    variant="outline"
                    size="sm"
                    spacing={0}
                    value={editorMode}
                    aria-labelledby="template-editor-mode-label"
                    onValueChange={(next) => {
                      if (next !== "visual" && next !== "html") return;
                      void switchEditorMode(next);
                    }}
                  >
                    <ToggleGroupItem value="html" className={toolbarToggleItem}>
                      HTML
                    </ToggleGroupItem>
                    <ToggleGroupItem value="visual" className={toolbarToggleItem}>
                      Visual
                    </ToggleGroupItem>
                  </ToggleGroup>
                </div>
                <VariableMenu entries={variableEntries} onInsert={insertVariable} />
              </div>

              {editorMode === "html" ? (
                <>
                  <CodeMirror
                    ref={codeMirrorRef}
                    value={htmlBody}
                    onChange={setHtmlBody}
                    theme={resolvedTheme === "light" ? "light" : "dark"}
                    extensions={[htmlLang()]}
                    minHeight="22rem"
                    placeholder="<p>Hello {{user.name}}</p>"
                    basicSetup={{ foldGutter: false }}
                    aria-label="Email HTML"
                    className="overflow-hidden rounded-lg border border-border text-sm focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/40"
                  />
                  <p className="text-xs text-muted-foreground">
                    A link or button href takes a variable the same way — type{" "}
                    <code className="font-mono">{"{{path}}"}</code> directly into it.
                  </p>
                </>
              ) : (
                <>
                  <div className="min-h-72 overflow-hidden rounded-lg border border-border bg-background focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/40">
                    <EditorProvider
                      key={editorRemountKey}
                      extensions={extensions}
                      content={(tiptapJson as JSONContent | undefined) ?? (htmlBody || undefined)}
                      editorProps={{
                        attributes: {
                          class: "cb-email-editor-content p-3 outline-none",
                          "aria-label": "Email body",
                        },
                      }}
                    >
                      <EditorInstanceCapture editorRef={editorInstanceRef} onUpdate={handleEditorUpdate} />
                      <BubbleMenu />
                      <SlashCommand items={SLASH_COMMAND_ITEMS} />
                    </EditorProvider>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    Type <code className="font-mono">/</code> for blocks, images and variables. Variables insert as a
                    chip and export as <code className="font-mono">{"{{path}}"}</code> text — a link/button URL is a
                    plain field, type the token directly into it.
                  </p>
                </>
              )}
            </section>

            <section aria-label="Plain-text alternative" className="flex flex-col gap-1.5">
              <div className="flex items-center justify-between gap-2">
                <Label htmlFor="template-text">Plain-text alternative</Label>
                <Button
                  type="button"
                  variant="ghost"
                  size="xs"
                  onClick={() => void currentEmail().then(({ html: h }) => setText(htmlToTextApprox(h)))}
                >
                  Auto-derive from body
                </Button>
              </div>
              <Textarea
                id="template-text"
                value={text}
                onChange={(e) => setText(e.target.value)}
                placeholder="Left blank, the server derives one automatically at send time."
                className="min-h-24 font-mono text-xs"
              />
            </section>
          </div>
        </div>

        <div className={cn("min-h-0 flex-col overflow-y-auto lg:flex", mobilePane === "panel" ? "flex" : "hidden")}>
          <Tabs value={sideTab} onValueChange={setSideTab} className="min-h-0 flex-1 gap-3 p-3 sm:p-page">
            <TabsList className="grid w-full grid-cols-4">
              <TabsTrigger value="preview">Preview</TabsTrigger>
              <TabsTrigger value="theme">Theme</TabsTrigger>
              <TabsTrigger value="variables">Variables</TabsTrigger>
              <TabsTrigger value="settings">Settings</TabsTrigger>
            </TabsList>

            <TabsContent value="preview" className="flex flex-col gap-3">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex items-center gap-2 text-xs text-muted-foreground" role="status">
                  {rendering ? (
                    <>
                      <Spinner className="size-3" />
                      Rendering…
                    </>
                  ) : previewError ? (
                    <>
                      <StatusDot tone="danger" />
                      Render failed
                    </>
                  ) : (
                    <>
                      <StatusDot tone="success" />
                      Live preview
                    </>
                  )}
                </div>
                <div className="flex items-center gap-1.5">
                  <ToggleGroup
                    type="single"
                    variant="outline"
                    size="sm"
                    spacing={0}
                    value={previewScheme}
                    aria-label="Preview colour scheme"
                    onValueChange={(next) => {
                      if (next === "light" || next === "dark") setPreviewScheme(next);
                    }}
                  >
                    <ToggleGroupItem value="light" aria-label="Light mode preview" title="Light mode preview">
                      <Sun />
                    </ToggleGroupItem>
                    <ToggleGroupItem value="dark" aria-label="Dark mode preview" title="Dark mode preview">
                      <Moon />
                    </ToggleGroupItem>
                  </ToggleGroup>
                  <ToggleGroup
                    type="single"
                    variant="outline"
                    size="sm"
                    spacing={0}
                    value={previewWidth}
                    aria-label="Preview width"
                    onValueChange={(next) => {
                      if (next === "desktop" || next === "mobile") setPreviewWidth(next);
                    }}
                  >
                    <ToggleGroupItem value="desktop" aria-label="Desktop width" title="Desktop width">
                      <Laptop />
                    </ToggleGroupItem>
                    <ToggleGroupItem value="mobile" aria-label="Mobile width (375px)" title="Mobile width (375px)">
                      <Smartphone />
                    </ToggleGroupItem>
                  </ToggleGroup>
                  <Button
                    type="button"
                    variant="outline"
                    size="icon-sm"
                    aria-label="Re-render preview"
                    title="Re-render preview"
                    onClick={() => setPreviewNonce((n) => n + 1)}
                  >
                    <RotateCw className={cn(rendering && "animate-spin")} />
                  </Button>
                </div>
              </div>

              {previewError ? (
                <ErrorState
                  compact
                  title="The preview couldn't render"
                  error={previewError}
                  onRetry={() => setPreviewNonce((n) => n + 1)}
                />
              ) : null}

              {preview ? (
                <div className={cn("flex flex-col gap-2", previewError && "opacity-60")}>
                  <div className="rounded-lg border border-border bg-card px-3 py-2 text-xs">
                    <span className="text-muted-foreground">Subject </span>
                    <span className="font-mono">{preview.subject || "(no subject)"}</span>
                  </div>
                  {/* The frame stands in for the recipient's mail client, so its
                      canvas colour follows the emulated scheme, not the admin theme. */}
                  <div
                    className="mx-auto w-full overflow-hidden rounded-lg border border-border transition-[max-width] duration-base ease-out-strong"
                    style={{
                      maxWidth: previewWidth === "mobile" ? 375 : "100%",
                      backgroundColor: previewScheme === "dark" ? "#18181b" : "#ffffff",
                    }}
                  >
                    <iframe title="Template preview" srcDoc={previewDoc} sandbox="" className="h-[36rem] w-full" />
                  </div>
                  {previewError ? (
                    <p className="text-center text-xs text-muted-foreground">Showing the last successful render.</p>
                  ) : null}
                </div>
              ) : previewError ? null : (
                <Skeleton className="h-[36rem] w-full" />
              )}

              <div className="flex flex-col gap-1.5">
                <Label htmlFor="template-sample-data">Sample data</Label>
                <Textarea
                  id="template-sample-data"
                  value={sampleData}
                  onChange={(e) => setSampleData(e.target.value)}
                  className="min-h-24 font-mono text-xs"
                  spellCheck={false}
                  aria-invalid={sampleParse.error !== null}
                  aria-describedby={sampleParse.error ? "template-sample-error" : "template-sample-help"}
                />
                {sampleParse.error ? (
                  <FieldError id="template-sample-error">{sampleParse.error}</FieldError>
                ) : (
                  <p id="template-sample-help" className="text-xs text-muted-foreground">
                    JSON the preview and test sends fill <code className="font-mono">{"{{variables}}"}</code> from.
                    Never saved with the template.
                  </p>
                )}
              </div>
            </TabsContent>

            <TabsContent value="theme" className="flex flex-col gap-3">
              {editorMode === "visual" ? (
                <ThemePanel value={themeState} onChange={updateTheme} />
              ) : (
                <EmptyState
                  compact
                  icon={<Palette />}
                  title="Theming is for the visual editor"
                  description="Colours, fonts and width apply to the visual editor's blocks. HTML templates style themselves."
                  action={
                    <Button variant="outline" size="sm" onClick={() => void switchEditorMode("visual")}>
                      Switch to visual
                    </Button>
                  }
                />
              )}
            </TabsContent>

            <TabsContent value="variables" className="flex flex-col gap-3">
              <p className="text-xs text-muted-foreground">
                Click one to insert <code className="font-mono">{"{{…}}"}</code> at the cursor. They come from the
                sample data, plus the always-available <code className="font-mono">appName</code> and{" "}
                <code className="font-mono">appUrl</code>. In Visual mode, <code className="font-mono">/variable</code>{" "}
                works too.
              </p>
              <ul className="flex flex-col gap-1">
                {variableEntries.map((entry) => (
                  <li key={entry.key}>
                    <button
                      type="button"
                      onClick={() => insertVariable(entry.key)}
                      className="flex w-full flex-col items-start gap-0.5 rounded-md border border-border bg-card px-2.5 py-1.5 text-left transition-colors duration-fast hover:bg-muted"
                    >
                      <code className="font-mono text-xs">{`{{${entry.key}}}`}</code>
                      <span className="text-xs text-muted-foreground">{entry.description}</span>
                    </button>
                  </li>
                ))}
              </ul>
            </TabsContent>

            <TabsContent value="settings" className="flex flex-col gap-5">
              <section className="flex flex-col gap-2">
                <h2 className="text-sm font-medium">Layout</h2>
                <div className="flex items-center justify-between gap-3 rounded-lg border border-border bg-card p-3">
                  <div className="flex flex-col gap-0.5">
                    <Label htmlFor="template-layout">Wrap in the branded layout</Label>
                    <p className="text-xs text-muted-foreground">
                      Adds the shared header and footer around this body.
                    </p>
                  </div>
                  <Switch id="template-layout" checked={layout} onCheckedChange={setLayout} />
                </div>
              </section>
              <section className="flex flex-col gap-2">
                <h2 className="text-sm font-medium">Access</h2>
                <RuleField
                  label="Who can send this template"
                  value={sendRule}
                  onChange={setSendRule}
                  nullOption={{
                    label: "Superusers",
                    description: "Only a superuser or API key may send this template.",
                  }}
                  publicOption={{
                    label: "Anyone",
                    description: "Any caller — including anonymous ones — may send this template. Use with care.",
                  }}
                />
              </section>
            </TabsContent>
          </Tabs>
        </div>
      </div>

      <AlertDialog open={confirmReset} onOpenChange={setConfirmReset}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard your changes?</AlertDialogTitle>
            <AlertDialogDescription>
              {isNew
                ? "The editor goes back to a blank template. Nothing has been created yet."
                : "The editor goes back to the last saved version of "}
              {isNew ? null : <span className="font-mono">{baseline?.key}</span>}
              {isNew ? null : ". Nothing on the server changes."}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={resetToBaseline}>
              Discard changes
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={blocker.status === "blocked"} onOpenChange={(open) => !open && blocker.reset?.()}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Leave with unsaved changes?</AlertDialogTitle>
            <AlertDialogDescription>
              Your edits to <span className="font-mono">{title}</span> haven't been saved. Leaving this page loses them.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel onClick={() => blocker.reset?.()}>Stay</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => blocker.proceed?.()}>
              Leave and lose them
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </PageShell>
  );
}
