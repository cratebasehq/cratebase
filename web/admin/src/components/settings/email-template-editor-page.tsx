import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useTheme } from "next-themes";
import { ArrowLeft, Laptop, Loader2, Send, Smartphone, Sparkles } from "lucide-react";
import { toast } from "sonner";
import CodeMirror, { type ReactCodeMirrorRef } from "@uiw/react-codemirror";
import { html as htmlLang } from "@codemirror/lang-html";
import { EmailEditor, type EmailEditorProps, type EmailEditorRef } from "@react-email/editor";
import "@react-email/editor/themes/default.css";
import "@react-email/editor/styles/bubble-menu.css";
import "@react-email/editor/styles/slash-command.css";
import "@react-email/editor/styles/inspector.css";
import { cb, currentSuperuser, describeFailure } from "@/lib/api";
import { useDevMailInboxAvailable } from "@/hooks/use-settings";
import { settingsEmailTemplateEditorRoute } from "@/routes/settings-email-template-editor";
import { RuleField } from "@/components/collections/rule-field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";

/** The visual editor's own document shape — re-derived from its own
 * exported types (`EmailEditorProps`/`EmailEditorRef`) rather than a
 * direct `@tiptap/core` import: that package is a dependency of
 * `@react-email/editor`, not of this app, and isn't guaranteed to be
 * resolvable as a standalone import from here. */
type EditorContent = EmailEditorProps["content"];
type EditorJson = ReturnType<EmailEditorRef["getJSON"]>;

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

const SAMPLE_DATA_PLACEHOLDER = `{\n  "user": { "name": "Ada" }\n}`;

/**
 * Create or edit one `_emailTemplates` row: subject/name/locale/layout,
 * the `sendRule` gate (`RuleField`, same component a collection API rule
 * uses), an HTML editor (CodeMirror) or the visual editor
 * (`@react-email/editor`, opaque `design` JSON — the server never reads
 * it, only the `html`/`text` it produces), a live preview rendered
 * through `POST /api/mails/preview` against editable sample data, and a
 * test send through `POST /api/mails/send`.
 */
export function EmailTemplateEditorPage() {
  const { id } = settingsEmailTemplateEditorRoute.useParams();
  const isNew = id === "new";
  const queryClient = useQueryClient();
  const { resolvedTheme } = useTheme();
  const { data: devMailAvailable } = useDevMailInboxAvailable();

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
  const [design, setDesign] = useState<EditorJson | null>(null);
  const [loadedOnce, setLoadedOnce] = useState(false);

  // Seed local form state once the record loads (or immediately for a
  // new template) — a query refetch afterwards must not clobber in-flight
  // edits.
  useEffect(() => {
    if (loadedOnce) return;
    if (isNew) {
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
    setDesign((existing.design as EditorJson | null) ?? null);
    setLoadedOnce(true);
  }, [existing, isNew, loadedOnce]);

  const codeMirrorRef = useRef<ReactCodeMirrorRef>(null);
  const emailEditorRef = useRef<EmailEditorRef>(null);
  const [visualTick, setVisualTick] = useState(0);

  const [sampleData, setSampleData] = useState(SAMPLE_DATA_PLACEHOLDER);
  const sampleDataKeys = useMemo(() => {
    try {
      return flattenKeys(JSON.parse(sampleData));
    } catch {
      return [];
    }
  }, [sampleData]);

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
    if (editorMode === "visual" && emailEditorRef.current) {
      try {
        return await emailEditorRef.current.getEmail();
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
      let finalDesign: EditorJson | null = design;
      if (editorMode === "visual" && emailEditorRef.current) {
        const email = await emailEditorRef.current.getEmail();
        finalHtml = email.html;
        finalText = email.text || text;
        finalDesign = emailEditorRef.current.getJSON();
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
        design: editorMode === "visual" ? finalDesign : design,
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

  const emailEditorContent: EditorContent = (design as EditorContent) ?? (htmlBody || undefined);

  if (!isNew && isLoading) {
    return (
      <div className="flex flex-col gap-4 p-page">
        <Skeleton className="h-8 w-64" />
        <Skeleton className="h-96 w-full" />
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
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
              <div className="flex flex-col">
                <Label htmlFor="template-layout">Wrap in the branded layout</Label>
                <p className="text-xs text-muted-foreground">Adds the shared header/footer chrome around this body.</p>
              </div>
              <Switch id="template-layout" checked={layout} onCheckedChange={setLayout} />
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
                  if (next === "visual" || next === "html") setEditorMode(next);
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
                  {sampleDataKeys.length > 0 ? (
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
                  ) : null}
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
              </div>
            ) : (
              <div className="flex flex-col gap-1.5">
                <div className="flex items-center justify-between">
                  <Label>Design</Label>
                  {sampleDataKeys.length > 0 ? (
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
                            onSelect={() => emailEditorRef.current?.editor?.commands.insertContent(`{{${k}}}`)}
                          >
                            <code className="font-mono text-xs">{`{{${k}}}`}</code>
                          </DropdownMenuItem>
                        ))}
                      </DropdownMenuContent>
                    </DropdownMenu>
                  ) : null}
                </div>
                <div className="min-h-72 rounded-md border border-border/60 bg-background p-2">
                  <EmailEditor
                    ref={emailEditorRef}
                    content={emailEditorContent}
                    placeholder="Start typing, or press '/' for blocks…"
                    onUpdate={() => setVisualTick((t) => t + 1)}
                  />
                </div>
                <p className="text-xs text-muted-foreground">
                  Variables insert as <code className="font-mono">{"{{path}}"}</code> text — they render once sent,
                  same as HTML mode.
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
          </div>
        </div>

        <div className="flex min-h-0 flex-col overflow-y-auto p-page">
          <div className="flex flex-col gap-3">
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
          </div>
        </div>
      </div>
    </div>
  );
}
