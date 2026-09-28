import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useTheme } from "next-themes";
import { ArrowLeft, Laptop, Loader2, Send, Smartphone, Sparkles } from "lucide-react";
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
import { useDevMailInboxAvailable, useSettings } from "@/hooks/use-settings";
import { settingsEmailTemplateEditorRoute } from "@/routes/settings-email-template-editor";
import { RuleField } from "@/components/collections/rule-field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { uploadEmailImage } from "@/components/settings/email-editor/image-upload";
import { Variable, variableKeysRef, variableSlashCommandItem } from "@/components/settings/email-editor/variable-extension";
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

/** Flattens a JSON value into dotted paths for the "Insert variable" menu
 * — `{ user: { name: "Ada" } }` becomes `["user.name"]`. Arrays are
 * skipped (a `{{var}}` path has no index syntax). */
function flattenKeys(value: unknown, prefix = ""): string[] {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return prefix ? [prefix] : [];
  }
  const out: string[] = [];
  for (const [key, v] of Object.entries(value as Record<string, unknown>)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (v !== null && typeof v === "object" && !Array.isArray(v)) {
      out.push(...flattenKeys(v, path));
    } else {
      out.push(path);
    }
  }
  return out;
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
  editorRef: React.MutableRefObject<Editor | null>;
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
  const { resolvedTheme } = useTheme();
  const { data: devMailAvailable } = useDevMailInboxAvailable();
  const { data: settings } = useSettings();
  const appUrl = settings?.meta.appURL ?? "";

  const { data: existing, isLoading } = useQuery({
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

  // Seed local form state once the record loads (or immediately for a
  // new template) — a query refetch afterwards must not clobber in-flight
  // edits.
  useEffect(() => {
    if (loadedOnce) return;
    if (isNew) {
      setThemeState(defaultEmailTheme(settings?.meta.brandColor));
      setLoadedOnce(true);
      return;
    }
    if (!existing) return;
    setKey(existing.key);
    setName(existing.name);
    setSubject(existing.subject);
    setLocaleValue(existing.locale);
    setLayout(existing.layout);
    setHtmlBody(existing.html);
    setText(existing.text);
    setSendRule(existing.sendRule);
    setEditorMode(existing.editor === "visual" ? "visual" : "html");
    const { tiptap, theme } = loadDesign(existing.design, settings?.meta.brandColor);
    setTiptapJson(tiptap);
    setThemeState(theme);
    setLoadedOnce(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only re-seed once existing/settings first arrive
  }, [existing, isNew, loadedOnce, settings]);

  const codeMirrorRef = useRef<ReactCodeMirrorRef>(null);
  const editorInstanceRef = useRef<Editor | null>(null);
  const [visualTick, setVisualTick] = useState(0);

  const [sampleData, setSampleData] = useState(SAMPLE_DATA_PLACEHOLDER);
  const sampleDataKeys = useMemo(() => {
    try {
      return [...flattenKeys(JSON.parse(sampleData)), "appName", "appUrl"];
    } catch {
      return ["appName", "appUrl"];
    }
  }, [sampleData]);
  // The Variable extension's chip/slash-command UI reads this ref live
  // rather than an extension option, so editing the sample data never
  // needs to reconfigure (and therefore remount) the editor.
  variableKeysRef.current = sampleDataKeys;

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
  const editorRemountKey = JSON.stringify(themeState);
  function updateTheme(next: EmailThemeState) {
    const latest = editorInstanceRef.current?.getJSON();
    if (latest) setTiptapJson(latest);
    setThemeState(next);
  }

  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [previewWidth, setPreviewWidth] = useState<"desktop" | "mobile">("desktop");

  const [testTo, setTestTo] = useState(currentSuperuser()?.email ?? "");
  const testSend = useMutation({
    mutationFn: async () => {
      const { html: sendHtml, text: sendText } = await currentEmail();
      const data = parseSampleData();
      return cb.mails.send({
        to: testTo,
        subject,
        html: sendHtml,
        text: sendText || undefined,
        data,
        locale: localeValue || undefined,
      });
    },
    onSuccess: (result) => {
      if (result.status === "failed") {
        toast.error("Test send failed", { description: result.error });
      } else {
        toast.success(devMailAvailable ? "Test sent — check the dev mail inbox" : "Test sent");
      }
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  function parseSampleData(): Record<string, unknown> {
    try {
      const parsed = sampleData.trim() ? JSON.parse(sampleData) : {};
      return typeof parsed === "object" && parsed !== null ? (parsed as Record<string, unknown>) : {};
    } catch {
      return {};
    }
  }

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
        const { html: previewHtml, text: previewText } = await currentEmail();
        const data = parseSampleData();
        try {
          const result = await cb.mails.preview({
            subject,
            html: previewHtml,
            text: previewText || undefined,
            data,
            locale: localeValue || undefined,
          });
          if (!cancelled) {
            setPreview(result);
            setPreviewError(null);
          }
        } catch (failure) {
          if (!cancelled) setPreviewError(describeFailure(failure).detail);
        }
      })();
    }, 400);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- currentEmail/parseSampleData read live state via closures
  }, [subject, htmlBody, text, sampleData, localeValue, visualTick, editorMode]);

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
      return isNew
        ? cb.collection("_emailTemplates").create(body)
        : cb.collection("_emailTemplates").update(id, body);
    },
    onSuccess: (saved) => {
      toast.success(isNew ? "Template created" : "Template saved");
      void queryClient.invalidateQueries({ queryKey: ["email-templates"] });
      const record = saved as unknown as TemplateRecord;
      void queryClient.invalidateQueries({ queryKey: ["email-templates", record.id] });
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  function pickStarter(template: StarterTemplate) {
    setTiptapJson(template.content);
    if (template.subject) setSubject(template.subject);
    setEditorMode("visual");
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
      const composed = await composeReactEmail({ editor: editorInstanceRef.current });
      setHtmlBody(composed.html);
      setTiptapJson(null);
    } else if (editorMode === "html") {
      setTiptapJson(null);
    }
    setEditorMode(next);
  }

  // Gate on `loadedOnce`, not just `isLoading`: react-query resolves
  // `isLoading` to `false` the instant `existing` arrives, one render
  // before the "seed local state" effect above has actually run — the
  // visual editor must never mount before `tiptapJson` reflects the
  // loaded row, since nothing here forces a remount just because state
  // changed (only a theme change does, via `editorRemountKey`).
  if (!isNew && (isLoading || !loadedOnce)) {
    return (
      <div className="flex flex-col gap-4 p-page">
        <Skeleton className="h-8 w-64" />
        <Skeleton className="h-96 w-full" />
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      {isNew ? (
        <StarterGalleryDialog open={!starterPicked} onPick={pickStarter} />
      ) : null}

      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border px-page py-3">
        <div className="flex min-w-0 items-center gap-2">
          <Button variant="ghost" size="icon-sm" asChild>
            <Link to="/settings/email-templates">
              <ArrowLeft className="size-4" />
            </Link>
          </Button>
          <div className="flex min-w-0 flex-col">
            <Input
              value={key}
              onChange={(e) => setKey(e.target.value)}
              placeholder="template-key"
              className="h-7 w-56 border-none bg-transparent px-0 font-mono text-sm font-medium shadow-none focus-visible:ring-0"
            />
          </div>
        </div>
        <Button size="sm" disabled={save.isPending || !key || !subject} onClick={() => save.mutate()}>
          {save.isPending ? <Spinner className="size-3.5" /> : null}
          {isNew ? "Create" : "Save"}
        </Button>
      </div>

      <div className="grid min-h-0 flex-1 grid-cols-1 gap-0 overflow-hidden lg:grid-cols-[1fr_420px]">
        <div className="flex min-h-0 flex-col overflow-y-auto border-r border-border p-page">
          <div className="flex flex-col gap-4">
            <div className="grid grid-cols-2 gap-3">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="template-name">Name</Label>
                <Input id="template-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Welcome email" />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="template-locale">Locale</Label>
                <Input
                  id="template-locale"
                  value={localeValue}
                  onChange={(e) => setLocaleValue(e.target.value)}
                  placeholder="(default — blank)"
                  className="font-mono text-sm"
                />
              </div>
            </div>

            <div className="flex flex-col gap-1.5">
              <Label htmlFor="template-subject">Subject</Label>
              <Input
                id="template-subject"
                value={subject}
                onChange={(e) => setSubject(e.target.value)}
                placeholder="Welcome to {{appName}}!"
                className="font-mono text-sm"
              />
            </div>

            <div className="flex items-center justify-between">
              <Label>Editor</Label>
              <ToggleGroup
                type="single"
                variant="outline"
                size="sm"
                spacing={0}
                value={editorMode}
                onValueChange={(next) => {
                  if (next !== "visual" && next !== "html") return;
                  void switchEditorMode(next);
                }}
              >
                <ToggleGroupItem value="html">HTML</ToggleGroupItem>
                <ToggleGroupItem value="visual">Visual</ToggleGroupItem>
              </ToggleGroup>
            </div>

            {editorMode === "html" ? (
              <div className="flex flex-col gap-1.5">
                <div className="flex items-center justify-between">
                  <Label>HTML</Label>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button variant="ghost" size="sm" className="h-6 gap-1 text-xs">
                        <Sparkles className="size-3" />
                        Insert variable
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      {sampleDataKeys.map((k) => (
                        <DropdownMenuItem
                          key={k}
                          onSelect={() => insertAtCursor(codeMirrorRef.current?.view, `{{${k}}}`)}
                        >
                          <code className="font-mono text-xs">{`{{${k}}}`}</code>
                        </DropdownMenuItem>
                      ))}
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
                <CodeMirror
                  ref={codeMirrorRef}
                  value={htmlBody}
                  onChange={setHtmlBody}
                  theme={resolvedTheme === "light" ? "light" : "dark"}
                  extensions={[htmlLang()]}
                  minHeight="18rem"
                  placeholder="<p>Hello {{user.name}}</p>"
                  basicSetup={{ foldGutter: false }}
                  className="overflow-hidden rounded-md border border-border/60 text-sm"
                />
                <p className="text-xs text-muted-foreground">
                  A link or button href takes a variable the same way — type{" "}
                  <code className="font-mono">{"{{path}}"}</code> directly into it.
                </p>
              </div>
            ) : (
              <div className="flex flex-col gap-1.5">
                <div className="flex items-center justify-between">
                  <Label>Design</Label>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button variant="ghost" size="sm" className="h-6 gap-1 text-xs">
                        <Sparkles className="size-3" />
                        Insert variable
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      {sampleDataKeys.map((k) => (
                        <DropdownMenuItem
                          key={k}
                          onSelect={() => editorInstanceRef.current?.commands.insertVariable(k)}
                        >
                          <code className="font-mono text-xs">{`{{${k}}}`}</code>
                        </DropdownMenuItem>
                      ))}
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
                <div className="min-h-72 overflow-hidden rounded-md border border-border/60 bg-background">
                  <EditorProvider
                    key={editorRemountKey}
                    extensions={extensions}
                    content={(tiptapJson as JSONContent | undefined) ?? (htmlBody || undefined)}
                    editorProps={{ attributes: { class: "cb-email-editor-content p-3 outline-none" } }}
                  >
                    <EditorInstanceCapture editorRef={editorInstanceRef} onUpdate={() => setVisualTick((t) => t + 1)} />
                    <BubbleMenu />
                    <SlashCommand items={SLASH_COMMAND_ITEMS} />
                  </EditorProvider>
                </div>
                <p className="text-xs text-muted-foreground">
                  Variables insert as a chip and export as{" "}
                  <code className="font-mono">{"{{path}}"}</code> text — a link/button URL is a
                  plain field, type the token directly into it.
                </p>
              </div>
            )}

            <div className="flex flex-col gap-1.5">
              <div className="flex items-center justify-between">
                <Label htmlFor="template-text">Plain-text alternative</Label>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-6 text-xs"
                  onClick={() => void currentEmail().then(({ html: h }) => setText(htmlToTextApprox(h)))}
                >
                  Auto-derive from HTML
                </Button>
              </div>
              <Textarea
                id="template-text"
                value={text}
                onChange={(e) => setText(e.target.value)}
                placeholder="Left blank, the server derives one automatically at send time."
                className="min-h-24 font-mono text-xs"
              />
            </div>
          </div>
        </div>

        <div className="flex min-h-0 flex-col overflow-y-auto p-page">
          <Tabs defaultValue="theme" className="flex min-h-0 flex-1 flex-col gap-3">
            <TabsList className="grid w-full grid-cols-4">
              <TabsTrigger value="theme">Theme</TabsTrigger>
              <TabsTrigger value="variables">Variables</TabsTrigger>
              <TabsTrigger value="settings">Settings</TabsTrigger>
              <TabsTrigger value="preview">Preview</TabsTrigger>
            </TabsList>

            <TabsContent value="theme" className="flex-1">
              {editorMode === "visual" ? (
                <ThemePanel value={themeState} onChange={updateTheme} />
              ) : (
                <p className="text-sm text-muted-foreground">
                  Theming applies to the visual editor. Switch to Visual mode to customize it.
                </p>
              )}
            </TabsContent>

            <TabsContent value="variables" className="flex flex-col gap-2">
              <p className="text-xs text-muted-foreground">
                From the sample data below, plus the always-available <code className="font-mono">appName</code>/
                <code className="font-mono">appUrl</code>. In Visual mode, also available via the{" "}
                <code className="font-mono">/variable</code> slash command.
              </p>
              <div className="flex flex-col gap-1">
                {sampleDataKeys.map((k) => (
                  <button
                    key={k}
                    type="button"
                    onClick={() => {
                      if (editorMode === "visual") editorInstanceRef.current?.commands.insertVariable(k);
                      else insertAtCursor(codeMirrorRef.current?.view, `{{${k}}}`);
                    }}
                    className="flex items-center justify-between rounded-md border border-border/60 px-2 py-1.5 text-left text-xs hover:bg-accent"
                  >
                    <code className="font-mono">{`{{${k}}}`}</code>
                    <Sparkles className="size-3 text-muted-foreground" />
                  </button>
                ))}
              </div>
            </TabsContent>

            <TabsContent value="settings" className="flex flex-col gap-4">
              <div className="flex items-center justify-between">
                <div className="flex flex-col">
                  <Label htmlFor="template-layout">Wrap in the branded layout</Label>
                  <p className="text-xs text-muted-foreground">Adds the shared header/footer chrome around this body.</p>
                </div>
                <Switch id="template-layout" checked={layout} onCheckedChange={setLayout} />
              </div>
              <RuleField
                label="Who can send this template"
                value={sendRule}
                onChange={setSendRule}
                nullOption={{ label: "Superusers", description: "Only a superuser or API key may send this template." }}
                publicOption={{
                  label: "Anyone",
                  description: "Any caller — including anonymous ones — may send this template. Use with care.",
                }}
              />
            </TabsContent>

            <TabsContent value="preview" className="flex min-h-0 flex-1 flex-col gap-3">
              <div className="flex items-center justify-between">
                <Label>Sample data</Label>
                <ToggleGroup
                  type="single"
                  variant="outline"
                  size="sm"
                  spacing={0}
                  value={previewWidth}
                  onValueChange={(next) => {
                    if (next === "desktop" || next === "mobile") setPreviewWidth(next);
                  }}
                >
                  <ToggleGroupItem value="desktop" aria-label="Desktop width">
                    <Laptop className="size-3.5" />
                  </ToggleGroupItem>
                  <ToggleGroupItem value="mobile" aria-label="Mobile width">
                    <Smartphone className="size-3.5" />
                  </ToggleGroupItem>
                </ToggleGroup>
              </div>
              <Textarea
                value={sampleData}
                onChange={(e) => setSampleData(e.target.value)}
                className="min-h-20 font-mono text-xs"
                spellCheck={false}
              />

              <Label>Preview</Label>
              <div
                className="mx-auto w-full overflow-hidden rounded-md border border-border bg-white transition-all"
                style={{ maxWidth: previewWidth === "mobile" ? 375 : "100%" }}
              >
                {previewError ? (
                  <p className="p-3 text-xs text-destructive">{previewError}</p>
                ) : preview ? (
                  <>
                    <div className="border-b border-border/60 bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
                      {preview.subject || "(no subject)"}
                    </div>
                    <iframe
                      title="Template preview"
                      srcDoc={preview.html}
                      sandbox=""
                      className="h-[520px] w-full"
                    />
                  </>
                ) : (
                  <div className="flex h-[520px] items-center justify-center text-muted-foreground">
                    <Loader2 className="size-4 animate-spin" />
                  </div>
                )}
              </div>

              <div className="flex flex-col gap-1.5 rounded-lg border border-border bg-card p-3">
                <Label htmlFor="template-test-to">Send a test</Label>
                <div className="flex gap-1.5">
                  <Input
                    id="template-test-to"
                    value={testTo}
                    onChange={(e) => setTestTo(e.target.value)}
                    placeholder="you@example.com"
                    className="h-control-sm flex-1 text-sm"
                  />
                  <Button
                    size="sm"
                    className="gap-1.5"
                    disabled={testSend.isPending || !testTo}
                    onClick={() => testSend.mutate()}
                  >
                    {testSend.isPending ? <Spinner className="size-3.5" /> : <Send className="size-3.5" />}
                    Send
                  </Button>
                </div>
                {devMailAvailable ? (
                  <Link to="/settings/mail-inbox" className="text-xs text-muted-foreground hover:underline">
                    No real SMTP configured — test sends land in the dev mail inbox →
                  </Link>
                ) : null}
              </div>
            </TabsContent>
          </Tabs>
        </div>
      </div>
    </div>
  );
}
