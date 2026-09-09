import { useId } from "react";
import { Link } from "@tanstack/react-router";
import { Check, Copy, ExternalLink, Plus, Trash2, TriangleAlert } from "lucide-react";
import {
  emptyOAuth2Provider,
  type AuthOptionsValue,
  type AuthProviderValue,
} from "@/lib/collection-form-value";
import { useSettings } from "@/hooks/use-settings";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";

/** A labeled control with help text underneath — the same shape
 * `OptionField` in `schema-field-row.tsx` uses for a field's type-specific
 * options, kept local here since that one isn't exported. */
function OptionField({
  label,
  help,
  children,
  className,
}: {
  label: string;
  help?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <label className={className ? `flex min-w-0 flex-col gap-1 ${className}` : "flex min-w-0 flex-col gap-1"}>
      <span className="text-xs font-medium text-foreground/80">{label}</span>
      {children}
      {help ? <span className="text-2xs leading-snug text-muted-foreground">{help}</span> : null}
    </label>
  );
}

function ToggleField({
  checked,
  onChange,
  label,
  help,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  help?: string;
}) {
  const id = useId();
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-2">
        <Switch id={id} checked={checked} onCheckedChange={onChange} />
        <label htmlFor={id} className="cursor-pointer text-sm text-foreground">
          {label}
        </label>
      </div>
      {help ? <span className="text-2xs leading-snug text-muted-foreground">{help}</span> : null}
    </div>
  );
}

function DurationField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <OptionField label={label}>
      <div className="flex items-center gap-2">
        <Input
          type="number"
          min={1}
          value={value}
          onChange={(e) => onChange(e.target.value === "" ? 1 : Number(e.target.value))}
          className="h-control-md w-28 font-tabular text-sm"
        />
        <span className="text-xs text-muted-foreground">seconds</span>
      </div>
    </OptionField>
  );
}

/** Where to get a client id/secret for the providers PocketBase (and
 * Cratebase) build endpoints in for — everything else is a "bring your
 * own OAuth2/OpenID app" case with no console to link to. */
const PROVIDER_GUIDES: Record<string, { console: string; consoleLabel: string; steps: string }> = {
  google: {
    console: "https://console.cloud.google.com/apis/credentials",
    consoleLabel: "Google Cloud Console → Credentials",
    steps: 'Create Credentials → OAuth client ID → Application type "Web application", then paste the callback URL below under "Authorized redirect URIs".',
  },
  github: {
    console: "https://github.com/settings/developers",
    consoleLabel: "GitHub → Settings → Developer settings → OAuth Apps",
    steps: 'New OAuth App, then paste the callback URL below into "Authorization callback URL".',
  },
  gitlab: {
    console: "https://gitlab.com/-/user_settings/applications",
    consoleLabel: "GitLab → User Settings → Applications",
    steps: "Add new application, check the scopes this collection needs, then paste the callback URL below into the Redirect URI field.",
  },
  discord: {
    console: "https://discord.com/developers/applications",
    consoleLabel: "Discord Developer Portal → Applications",
    steps: 'New Application → OAuth2 → paste the callback URL below into "Redirects".',
  },
  microsoft: {
    console: "https://portal.azure.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade",
    consoleLabel: "Azure Portal → App registrations",
    steps: 'New registration → Authentication → Add a platform → Web, then paste the callback URL below into "Redirect URIs".',
  },
};

/** The exact URL a provider redirects back to once someone approves the
 * sign-in — `crates/server/src/routes/oauth2_flow.rs`'s `callback_url`,
 * reconstructed client-side the same way. Every provider's console wants
 * this pasted in byte-for-byte (they reject a mismatch), which is why
 * this is a copy button next to a read-only field, not something to
 * retype. */
function CallbackUrlField({
  appURL,
  collectionName,
  providerName,
}: {
  appURL: string;
  collectionName: string;
  providerName: string;
}) {
  const { copy, copied } = useCopyToClipboard();
  const guide = PROVIDER_GUIDES[providerName.trim().toLowerCase()];

  if (!appURL) {
    return (
      <div className="flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/[0.06] px-3 py-2 text-xs sm:col-span-2">
        <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-warning" />
        <span>
          Set the app URL in{" "}
          <Link to="/settings/application" className="underline underline-offset-2 hover:no-underline">
            Settings → Application
          </Link>{" "}
          first — the redirect callback URL a provider needs is built from it, and the server refuses to start
          this flow (<code className="font-mono">app_url_not_configured</code>) until it's set.
        </span>
      </div>
    );
  }

  const callbackUrl = `${appURL.replace(/\/+$/, "")}/api/collections/${encodeURIComponent(
    collectionName || "COLLECTION_NAME",
  )}/oauth2/${encodeURIComponent(providerName || "PROVIDER_NAME")}/callback`;

  return (
    <div className="flex flex-col gap-1.5 sm:col-span-2">
      <OptionField
        label="Callback URL"
        help={
          guide ? (
            <>
              Paste this into {guide.consoleLabel}. {guide.steps}
            </>
          ) : (
            "The URL this provider redirects back to once someone approves the sign-in — paste it into that provider's own app settings."
          )
        }
      >
        <div className="relative">
          <Input
            readOnly
            value={callbackUrl}
            onFocus={(e) => e.currentTarget.select()}
            className="h-control-md pr-9 font-mono text-xs"
          />
          <button
            type="button"
            onClick={() => void copy(callbackUrl)}
            aria-label={copied ? "Copied" : "Copy callback URL"}
            className="absolute right-1 top-1/2 grid size-control-xs -translate-y-1/2 place-items-center rounded text-muted-foreground transition-colors hover:text-foreground"
          >
            {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
          </button>
        </div>
      </OptionField>
      {guide ? (
        <a
          href={guide.console}
          target="_blank"
          rel="noreferrer"
          className="inline-flex w-fit items-center gap-1 text-2xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
        >
          Open {guide.consoleLabel}
          <ExternalLink className="size-3" />
        </a>
      ) : null}
    </div>
  );
}

function ProviderRow({
  provider,
  onChange,
  onRemove,
  appURL,
  collectionName,
}: {
  provider: AuthProviderValue;
  onChange: (next: AuthProviderValue) => void;
  onRemove: () => void;
  appURL: string;
  collectionName: string;
}) {
  function patch(next: Partial<AuthProviderValue>) {
    onChange({ ...provider, ...next });
  }

  return (
    <div className="flex flex-col gap-3 rounded-lg border border-border bg-card p-3">
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <CallbackUrlField appURL={appURL} collectionName={collectionName} providerName={provider.name} />
        <OptionField label="Provider name" help='Built-in presets ("google", "github", …) or your own for a custom OpenID/OAuth2 endpoint.'>
          <Input
            value={provider.name}
            onChange={(e) => patch({ name: e.target.value })}
            placeholder="google"
            className="h-control-md font-mono text-sm"
          />
        </OptionField>
        <OptionField label="Display name" help="Shown on the sign-in button. Falls back to the provider name if left blank.">
          <Input
            value={provider.displayName}
            onChange={(e) => patch({ displayName: e.target.value })}
            placeholder="Google"
            className="h-control-md text-sm"
          />
        </OptionField>
        <OptionField label="Client id">
          <Input
            value={provider.clientId}
            onChange={(e) => patch({ clientId: e.target.value })}
            className="h-control-md font-mono text-sm"
          />
        </OptionField>
        <OptionField
          label="Client secret"
          help="The server never returns a stored secret, so this box starts empty for an existing provider — leave it blank only if you don't want to change it. Every other field on this row is safe to edit without retyping it, but if you don't have it handy, cancel out of this form and come back once you do."
        >
          <Input
            type="password"
            value={provider.clientSecret}
            onChange={(e) => patch({ clientSecret: e.target.value })}
            autoComplete="new-password"
            placeholder="Stored — leave blank to keep"
            className="h-control-md text-sm"
          />
        </OptionField>
        <OptionField label="Auth URL">
          <Input
            value={provider.authURL}
            onChange={(e) => patch({ authURL: e.target.value })}
            placeholder="https://accounts.example.com/oauth2/authorize"
            className="h-control-md font-mono text-xs"
          />
        </OptionField>
        <OptionField label="Token URL">
          <Input
            value={provider.tokenURL}
            onChange={(e) => patch({ tokenURL: e.target.value })}
            placeholder="https://accounts.example.com/oauth2/token"
            className="h-control-md font-mono text-xs"
          />
        </OptionField>
        <OptionField label="User info URL" className="sm:col-span-2">
          <Input
            value={provider.userInfoURL}
            onChange={(e) => patch({ userInfoURL: e.target.value })}
            placeholder="https://accounts.example.com/oauth2/userinfo"
            className="h-control-md font-mono text-xs"
          />
        </OptionField>
      </div>
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          <Checkbox
            id={`${provider.name || "provider"}-pkce`}
            checked={provider.pkce === true}
            onCheckedChange={(checked) => patch({ pkce: checked === true ? true : provider.pkce === false ? false : null })}
          />
          <label htmlFor={`${provider.name || "provider"}-pkce`} className="cursor-pointer text-xs text-muted-foreground">
            Force PKCE (leave unchecked to auto-detect from the provider)
          </label>
        </div>
        <Button type="button" variant="ghost" size="sm" className="h-control-sm gap-1.5 text-destructive" onClick={onRemove}>
          <Trash2 className="size-3.5" />
          Remove
        </Button>
      </div>
    </div>
  );
}

/**
 * Everything `cratebase_core::collection::AuthOptions` adds to an auth
 * collection beyond its fields and API rules: how a superuser signs
 * everyone else in (password, OAuth2, OTP, MFA) and how long each kind of
 * token this collection issues stays valid. PocketBase puts all of this on
 * the collection, not in the app-wide settings — a `posts` collection's
 * rate limit is global, but whether `users` accepts GitHub logins is not.
 */
export function AuthOptionsEditor({
  value,
  onChange,
  collectionName,
}: {
  value: AuthOptionsValue;
  onChange: (next: AuthOptionsValue) => void;
  collectionName: string;
}) {
  const { data: settings } = useSettings();
  const appURL = settings?.meta.appURL ?? "";

  function patch(next: Partial<AuthOptionsValue>) {
    onChange({ ...value, ...next });
  }

  function addProvider() {
    patch({ oauth2Providers: [...value.oauth2Providers, emptyOAuth2Provider()] });
  }

  function updateProvider(index: number, next: AuthProviderValue) {
    const providers = [...value.oauth2Providers];
    providers[index] = next;
    patch({ oauth2Providers: providers });
  }

  function removeProvider(index: number) {
    patch({ oauth2Providers: value.oauth2Providers.filter((_, i) => i !== index) });
  }

  return (
    <section className="flex flex-col gap-4">
      <div className="flex flex-col">
        <span className="text-sm font-medium text-foreground">Authentication</span>
        <span className="text-xs text-muted-foreground">
          How this collection's records sign in, and how long the tokens it issues last.
        </span>
      </div>

      <ToggleField
        checked={value.passwordAuthEnabled}
        onChange={(passwordAuthEnabled) => patch({ passwordAuthEnabled })}
        label="Password authentication"
        help="Sign in with the identity field above and a password. Turning this off still allows OAuth2/OTP sign-in if enabled below."
      />

      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <ToggleField
            checked={value.oauth2Enabled}
            onChange={(oauth2Enabled) => patch({ oauth2Enabled })}
            label="OAuth2"
            help="Sign in through a third-party provider."
          />
          <Button type="button" variant="outline" size="sm" className="h-control-sm gap-1.5" onClick={addProvider}>
            <Plus className="size-3.5" />
            Add provider
          </Button>
        </div>
        <p className="text-2xs leading-snug text-muted-foreground">
          Provider name "google" or "github" only needs a client id and secret — their endpoints are built in.
          Anything else is a custom OAuth2/OpenID provider and needs its own auth/token/user info URLs below.
        </p>
        {value.oauth2Providers.length > 0 ? (
          <div className="flex flex-col gap-2">
            {value.oauth2Providers.map((provider, i) => (
              <ProviderRow
                key={i}
                provider={provider}
                onChange={(next) => updateProvider(i, next)}
                onRemove={() => removeProvider(i)}
                appURL={appURL}
                collectionName={collectionName}
              />
            ))}
          </div>
        ) : null}
      </div>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <div className="flex flex-col gap-2 rounded-lg border border-border p-3">
          <ToggleField
            checked={value.mfaEnabled}
            onChange={(mfaEnabled) => patch({ mfaEnabled })}
            label="Multi-factor authentication"
            help="Require a second successful auth method before a session is issued."
          />
          {value.mfaEnabled ? (
            <>
              <DurationField label="Window to complete the second factor" value={value.mfaDuration} onChange={(mfaDuration) => patch({ mfaDuration })} />
              <OptionField label="Rule" help='A filter expression — MFA is required only when it matches the sign-in request. Empty means every sign-in.'>
                <Input
                  value={value.mfaRule}
                  onChange={(e) => patch({ mfaRule: e.target.value })}
                  placeholder="Leave blank to require MFA for every sign-in"
                  className="h-control-md font-mono text-sm"
                />
              </OptionField>
            </>
          ) : null}
        </div>

        <div className="flex flex-col gap-2 rounded-lg border border-border p-3">
          <ToggleField
            checked={value.otpEnabled}
            onChange={(otpEnabled) => patch({ otpEnabled })}
            label="One-time password"
            help="Sign in with a code emailed to the identity field, instead of a stored password."
          />
          {value.otpEnabled ? (
            <>
              <DurationField label="Code validity" value={value.otpDuration} onChange={(otpDuration) => patch({ otpDuration })} />
              <OptionField label="Code length">
                <Input
                  type="number"
                  min={4}
                  value={value.otpLength}
                  onChange={(e) => patch({ otpLength: e.target.value === "" ? 4 : Number(e.target.value) })}
                  className="h-control-md w-24 font-tabular text-sm"
                />
              </OptionField>
            </>
          ) : null}
        </div>
      </div>

      <div className="flex flex-col gap-3">
        <div className="flex flex-col">
          <span className="text-sm font-medium text-foreground">Token durations</span>
          <span className="text-xs text-muted-foreground">
            How long each kind of token this collection issues stays valid. Shortening one doesn't affect tokens
            already handed out — it only changes what gets minted next.
          </span>
        </div>
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
          <DurationField label="Auth token" value={value.authTokenDuration} onChange={(authTokenDuration) => patch({ authTokenDuration })} />
          <DurationField
            label="Password reset token"
            value={value.passwordResetTokenDuration}
            onChange={(passwordResetTokenDuration) => patch({ passwordResetTokenDuration })}
          />
          <DurationField
            label="Email change token"
            value={value.emailChangeTokenDuration}
            onChange={(emailChangeTokenDuration) => patch({ emailChangeTokenDuration })}
          />
          <DurationField
            label="Verification token"
            value={value.verificationTokenDuration}
            onChange={(verificationTokenDuration) => patch({ verificationTokenDuration })}
          />
          <DurationField label="File token" value={value.fileTokenDuration} onChange={(fileTokenDuration) => patch({ fileTokenDuration })} />
        </div>
      </div>
    </section>
  );
}
