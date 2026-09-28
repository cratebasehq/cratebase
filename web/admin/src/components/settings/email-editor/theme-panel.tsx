import { extendTheme, type EditorTheme, type ThemeComponentStyles, type ThemeConfig } from "@react-email/editor/plugins";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

/** Matches `cratebase_mailer::template::DEFAULT_BRAND_COLOR`. */
export const DEFAULT_BRAND_COLOR = "#171717";

const SYSTEM_FONT_STACK =
  "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif";

/** Everything the Theme tab lets an author customize, plus the base
 * built-in theme it starts from — persisted as `design.theme` (see
 * `email-template-editor-page.tsx`), a sibling of the editor's own
 * tiptap JSON, since `@react-email/editor` has no built-in "save my
 * theme choice with the document" story of its own that we've wired
 * up here. */
export interface EmailThemeState {
  base: EditorTheme;
  brandColor: string;
  fontFamily: string;
  backgroundColor: string;
  contentWidthPx: number;
  borderRadiusPx: number;
}

export const DEFAULT_EMAIL_THEME: EmailThemeState = {
  base: "basic",
  brandColor: DEFAULT_BRAND_COLOR,
  fontFamily: SYSTEM_FONT_STACK,
  backgroundColor: "#f4f4f5",
  contentWidthPx: 600,
  borderRadiusPx: 8,
};

/** The Theme tab's starting point for a new template: the instance's
 * own brand color (`settings.meta.brandColor`) when set, same default
 * otherwise — matching the brief's "default from settings.meta.
 * brandColor/logoUrl" (the logo itself isn't a theme property here;
 * it's the layout's own header, unaffected by this panel). */
export function defaultEmailTheme(brandColor: string | undefined): EmailThemeState {
  const trimmed = brandColor?.trim();
  return trimmed ? { ...DEFAULT_EMAIL_THEME, brandColor: trimmed } : DEFAULT_EMAIL_THEME;
}

function isValidState(state: EmailThemeState): state is EmailThemeState {
  return (
    typeof state === "object" &&
    state !== null &&
    (state.base === "basic" || state.base === "minimal") &&
    typeof state.brandColor === "string" &&
    typeof state.fontFamily === "string" &&
    typeof state.backgroundColor === "string" &&
    typeof state.contentWidthPx === "number" &&
    typeof state.borderRadiusPx === "number"
  );
}

/** Parses a saved `design.theme` value (untyped JSON off the wire),
 * falling back to [`defaultEmailTheme`] for anything missing or
 * malformed — an old row saved before this shipped has no `theme` at
 * all, and this should still open cleanly. */
export function parseEmailTheme(value: unknown, brandColor: string | undefined): EmailThemeState {
  if (value && typeof value === "object" && isValidState(value as EmailThemeState)) {
    return value as EmailThemeState;
  }
  return defaultEmailTheme(brandColor);
}

/** Builds the `@react-email/editor` theme (`EditorThemeInput`) the
 * live editor and `composeReactEmail` both use from our own state —
 * see `KnownThemeComponents`/`ThemeComponentStyles` in the package's
 * own `.d.ts`: `button`/`link`/`body`/`container` are real themeable
 * component keys, and each value is ordinary `React.CSSProperties`. */
export function buildEditorTheme(state: EmailThemeState): ThemeConfig {
  const overrides: ThemeComponentStyles = {
    button: {
      backgroundColor: state.brandColor,
      borderColor: state.brandColor,
      borderRadius: `${state.borderRadiusPx}px`,
    },
    link: { color: state.brandColor },
    body: {
      backgroundColor: state.backgroundColor,
      fontFamily: state.fontFamily,
    },
    container: {
      width: `${state.contentWidthPx}px`,
      borderRadius: `${state.borderRadiusPx}px`,
    },
  };
  return extendTheme(state.base, overrides);
}

const FONT_OPTIONS: { label: string; value: string }[] = [
  { label: "System sans-serif", value: SYSTEM_FONT_STACK },
  { label: "Serif", value: "Georgia, 'Times New Roman', serif" },
  { label: "Monospace", value: "'SFMono-Regular', ui-monospace, Menlo, Consolas, monospace" },
];

const RADIUS_PRESETS: { label: string; value: number }[] = [
  { label: "Square", value: 0 },
  { label: "Rounded", value: 8 },
  { label: "Pill", value: 24 },
];

const HEX_COLOR = /^#[0-9a-fA-F]{6}$/;

function ColorField({
  id,
  label,
  value,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      <div className="flex items-center gap-2">
        <input
          type="color"
          aria-label={`${label} swatch`}
          value={HEX_COLOR.test(value) ? value : "#000000"}
          onChange={(e) => onChange(e.target.value)}
          className="h-8 w-10 shrink-0 cursor-pointer rounded-md border border-border bg-transparent p-0.5"
        />
        <Input
          id={id}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="#1055c9"
          className="h-8 font-mono text-sm"
        />
      </div>
    </div>
  );
}

/**
 * The right sidebar's **Theme** tab: pick a built-in theme (Basic or
 * Minimal, `@react-email/editor`'s own two presets) and customize its
 * brand color, font, page background, content width, and corner
 * radius. Changing anything here is fed back to
 * `email-template-editor-page.tsx`, which rebuilds the theme via
 * [`buildEditorTheme`] and remounts the editor with it (the package's
 * own documented pattern for a live theme switch — see the Theming
 * docs' "Dynamic Switching" note).
 */
export function ThemePanel({
  value,
  onChange,
}: {
  value: EmailThemeState;
  onChange: (next: EmailThemeState) => void;
}) {
  function patch(next: Partial<EmailThemeState>) {
    onChange({ ...value, ...next });
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1.5">
        <Label>Base theme</Label>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          spacing={0}
          value={value.base}
          onValueChange={(next) => {
            if (next === "basic" || next === "minimal") patch({ base: next });
          }}
        >
          <ToggleGroupItem value="basic">Basic</ToggleGroupItem>
          <ToggleGroupItem value="minimal">Minimal</ToggleGroupItem>
        </ToggleGroup>
        <p className="text-xs text-muted-foreground">
          Basic ships full typography/spacing defaults; Minimal starts from almost nothing.
        </p>
      </div>

      <ColorField
        id="theme-brand-color"
        label="Brand color"
        value={value.brandColor}
        onChange={(brandColor) => patch({ brandColor })}
      />

      <div className="flex flex-col gap-1.5">
        <Label htmlFor="theme-font">Font</Label>
        <Select value={value.fontFamily} onValueChange={(fontFamily) => patch({ fontFamily })}>
          <SelectTrigger id="theme-font" className="h-8 text-sm">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {FONT_OPTIONS.map((f) => (
              <SelectItem key={f.value} value={f.value}>
                {f.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <ColorField
        id="theme-background"
        label="Page background"
        value={value.backgroundColor}
        onChange={(backgroundColor) => patch({ backgroundColor })}
      />

      <div className="flex flex-col gap-1.5">
        <Label htmlFor="theme-width">Content width</Label>
        <div className="flex items-center gap-2">
          <Input
            id="theme-width"
            type="number"
            min={320}
            max={800}
            step={10}
            value={value.contentWidthPx}
            onChange={(e) =>
              patch({ contentWidthPx: Number(e.target.value) || DEFAULT_EMAIL_THEME.contentWidthPx })
            }
            className="h-8 w-24 text-sm"
          />
          <span className="text-xs text-muted-foreground">px</span>
        </div>
      </div>

      <div className="flex flex-col gap-1.5">
        <Label>Corner radius</Label>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          spacing={0}
          value={String(value.borderRadiusPx)}
          onValueChange={(next) => {
            if (next) patch({ borderRadiusPx: Number(next) });
          }}
        >
          {RADIUS_PRESETS.map((r) => (
            <ToggleGroupItem key={r.value} value={String(r.value)}>
              {r.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>
    </div>
  );
}
