import { useId, useState } from "react";
import { Link } from "@tanstack/react-router";
import { Check, Copy, ExternalLink, KeyRound, Plus, Trash2, TriangleAlert } from "lucide-react";
import {
  emptyOAuth2Provider,
  isGenericOidcProvider,
  isJwtClientSecretProvider,
  KNOWN_PRESET_NAMES,
  type AuthEmailTemplateValue,
  type AuthOptionsValue,
  type AuthProviderValue,
} from "@/lib/collection-form-value";
import { useSettings } from "@/hooks/use-settings";
import { useCopyToClipboard } from "@/hooks/use-copy-to-clipboard";
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
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { RuleField } from "@/components/collections/rule-field";
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
  apple: {
    console: "https://developer.apple.com/account/resources/identifiers/list/serviceId",
    consoleLabel: "Apple Developer → Certificates, IDs & Profiles → Identifiers",
    steps:
      'Register a Services ID (its identifier is the "Client id" below), enable "Sign In with Apple", and paste the callback URL below into its website URLs\' "Return URLs". Then create a new Sign in with Apple key under Keys, and fill in Team ID / Key ID / the downloaded .p8 private key below — Apple has no static client secret at all.',
  },
  facebook: {
    console: "https://developers.facebook.com/apps",
    consoleLabel: "Meta for Developers → My Apps",
    steps: 'Add the Facebook Login product, then paste the callback URL below into "Valid OAuth Redirect URIs".',
  },
  twitter: {
    console: "https://developer.twitter.com/en/portal/projects-and-apps",
    consoleLabel: "X Developer Portal → Projects & Apps",
    steps:
      'Set up User authentication settings with OAuth 2.0, then paste the callback URL below into "Callback URI / Redirect URL". PKCE is mandatory for X — leave "Force PKCE" unchecked to use the default, which already applies it.',
  },
  linkedin: {
    console: "https://www.linkedin.com/developers/apps",
    consoleLabel: "LinkedIn Developers → My Apps",
    steps:
      'Add the "Sign In with LinkedIn using OpenID Connect" product, then paste the callback URL below into "Authorized redirect URLs".',
  },
  slack: {
    console: "https://api.slack.com/apps",
    consoleLabel: "Slack API → Your Apps",
    steps:
      'Add "Sign in with Slack" (OpenID Connect), then paste the callback URL below into its redirect URLs.',
  },
  twitch: {
    console: "https://dev.twitch.tv/console/apps",
    consoleLabel: "Twitch Developer Console → Applications",
    steps: 'Register your application, then paste the callback URL below into "OAuth Redirect URLs".',
  },
  spotify: {
    console: "https://developer.spotify.com/dashboard",
    consoleLabel: "Spotify for Developers → Dashboard",
    steps: 'Create an app, open its settings, and paste the callback URL below into "Redirect URIs".',
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
  function patchExtra(next: Record<string, string>) {
    patch({ extra: { ...provider.extra, ...next } });
  }

  const normalizedName = provider.name.trim().toLowerCase();
  const isApple = isJwtClientSecretProvider(provider.name);
  const isOidc = isGenericOidcProvider(provider.extra) || normalizedName.startsWith("oidc");
  const isKnownPreset = KNOWN_PRESET_NAMES.has(normalizedName);
  // A preset's (or a discovered OIDC issuer's) endpoints are never
  // hand-entered, so hiding the raw URL fields for them keeps the form
  // honest about what it's actually going to send.
  const needsRawUrls = !isApple && !isOidc && !isKnownPreset;

  return (
    <div className="flex flex-col gap-3 rounded-lg border border-border bg-card p-3">
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <CallbackUrlField appURL={appURL} collectionName={collectionName} providerName={provider.name} />
        <OptionField
          label="Provider name"
          help='A built-in preset name (see the list below), "oidc"/"oidc2"/"oidc3" for a generic issuer, or your own for a fully custom OAuth2 endpoint.'
        >
          <Input
            list="oauth2-known-provider-names"
            value={provider.name}
            onChange={(e) => patch({ name: e.target.value })}
            placeholder="google"
            className="h-control-md font-mono text-sm"
          />
          <datalist id="oauth2-known-provider-names">
            {[...KNOWN_PRESET_NAMES, "oidc"].map((name) => (
              <option key={name} value={name} />
            ))}
          </datalist>
        </OptionField>
        <OptionField label="Display name" help="Shown on the sign-in button. Falls back to the provider name if left blank.">
          <Input
            value={provider.displayName}
            onChange={(e) => patch({ displayName: e.target.value })}
            placeholder="Google"
            className="h-control-md text-sm"
          />
        </OptionField>
        <OptionField label={isApple ? "Client id (Service ID)" : "Client id"}>
          <Input
            value={provider.clientId}
            onChange={(e) => patch({ clientId: e.target.value })}
            placeholder={isApple ? "com.example.app.service" : undefined}
            className="h-control-md font-mono text-sm"
          />
        </OptionField>
        {isApple ? (
          <div className="flex items-start gap-2 rounded-lg border border-border bg-muted/30 px-3 py-2 text-2xs leading-snug text-muted-foreground sm:col-span-2">
            Apple has no static client secret — it's generated per request from the Team ID, Key ID, and private
            key below instead.
          </div>
        ) : (
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
        )}

        {isApple ? (
          <>
            <OptionField label="Team ID" help="Apple Developer → Membership.">
              <Input
                value={provider.extra.teamId ?? ""}
                onChange={(e) => patchExtra({ teamId: e.target.value })}
                placeholder="ABCDE12345"
                className="h-control-md font-mono text-sm"
              />
            </OptionField>
            <OptionField label="Key ID" help="The Sign in with Apple key's ID, from when it was created.">
              <Input
                value={provider.extra.keyId ?? ""}
                onChange={(e) => patchExtra({ keyId: e.target.value })}
                placeholder="ABCD123456"
                className="h-control-md font-mono text-sm"
              />
            </OptionField>
            <OptionField
              label="Private key (.p8)"
              className="sm:col-span-2"
              help="The server never returns a stored key, so this box starts empty for an existing provider — leave it blank only if you don't want to change it."
            >
              <textarea
                value={provider.extra.privateKey ?? ""}
                onChange={(e) => patchExtra({ privateKey: e.target.value })}
                placeholder={"-----BEGIN PRIVATE KEY-----\n…\n-----END PRIVATE KEY-----"}
                rows={4}
                className="w-full rounded-lg border border-border bg-background px-3 py-2 font-mono text-2xs"
              />
            </OptionField>
          </>
        ) : null}

        {normalizedName === "microsoft" ? (
          <OptionField
            label="Tenant"
            help='Defaults to "common" (personal + work/school accounts) when left blank. Use a specific tenant ID to restrict sign-in to one Entra ID organization.'
          >
            <Input
              value={provider.extra.tenant ?? ""}
              onChange={(e) => patchExtra({ tenant: e.target.value })}
              placeholder="common"
              className="h-control-md font-mono text-sm"
            />
          </OptionField>
        ) : null}

        {normalizedName === "gitlab" ? (
          <OptionField label="Base URL" help="Defaults to https://gitlab.com. Set this for a self-hosted GitLab instance.">
            <Input
              value={provider.extra.baseUrl ?? ""}
              onChange={(e) => patchExtra({ baseUrl: e.target.value })}
              placeholder="https://gitlab.com"
              className="h-control-md font-mono text-sm"
            />
          </OptionField>
        ) : null}

        {isOidc ? (
          <OptionField
            label="Issuer"
            className="sm:col-span-2"
            help="The provider's OIDC issuer URL — its /.well-known/openid-configuration, authorization/token/userinfo endpoints, and signing keys are discovered from this alone."
          >
            <Input
              value={provider.extra.issuer ?? ""}
              onChange={(e) => patchExtra({ issuer: e.target.value })}
              placeholder="https://issuer.example.com"
              className="h-control-md font-mono text-sm"
            />
          </OptionField>
        ) : null}

        {needsRawUrls ? (
          <>
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
          </>
        ) : null}
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

/** Subject + HTML body for one of this collection's own auth emails —
 * embedded directly on the collection (`cratebase_core::collection::
 * EmailTemplate`), distinct from the app-wide `_emailTemplates` collection
 * the Templates tab under Settings → Email edits with a visual editor.
 * There's no override-vs-link choice to make here: these auth flows
 * (verify/reset/magic link/login alert) always send this embedded
 * template, so editing it here — plain subject/body fields, not the rich
 * editor — is the only way to change their copy. */
function EmailTemplateFields({
  value,
  onChange,
  placeholders,
}: {
  value: AuthEmailTemplateValue;
  onChange: (next: AuthEmailTemplateValue) => void;
  placeholders: string;
}) {
  return (
    <div className="flex flex-col gap-2">
      <OptionField label="Subject">
        <Input
          value={value.subject}
          onChange={(e) => onChange({ ...value, subject: e.target.value })}
          className="h-control-md text-sm"
        />
      </OptionField>
      <OptionField label="Body (HTML)" help={`Placeholders: ${placeholders}`}>
        <textarea
          value={value.body}
          onChange={(e) => onChange({ ...value, body: e.target.value })}
          rows={5}
          className="w-full rounded-lg border border-border bg-background px-3 py-2 font-mono text-2xs"
        />
      </OptionField>
    </div>
  );
}

/** A token kind's duration plus a one-way "regenerate secret" action. The
 * server never returns a stored secret (there's nothing to show), so this
 * is only ever a write: checking the box and saving mints a fresh random
 * secret, which invalidates every token of that kind issued so far since
 * its signature no longer matches. Confirmed with a dialog because it logs
 * out every session at once — the same blast radius as changing every
 * collection member's password. */
function TokenDurationField({
  label,
  duration,
  onDurationChange,
  regenerate,
  onRegenerateChange,
}: {
  label: string;
  duration: number;
  onDurationChange: (duration: number) => void;
  regenerate: boolean;
  onRegenerateChange: (regenerate: boolean) => void;
}) {
  const [confirming, setConfirming] = useState(false);
  return (
    <div className="flex flex-col gap-1.5">
      <DurationField label={label} value={duration} onChange={onDurationChange} />
      {regenerate ? (
        <div className="flex items-center gap-1.5 rounded-md bg-warning/[0.08] px-2 py-1 text-2xs text-warning">
          <KeyRound className="size-3 shrink-0" />
          Will regenerate on save — every existing {label.toLowerCase()} stops working.
          <button type="button" className="ml-auto underline underline-offset-2" onClick={() => onRegenerateChange(false)}>
            Cancel
          </button>
        </div>
      ) : (
        <button
          type="button"
          onClick={() => setConfirming(true)}
          className="w-fit text-2xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
        >
          Regenerate secret…
        </button>
      )}
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Regenerate the {label.toLowerCase()} secret?</AlertDialogTitle>
            <AlertDialogDescription>
              A new random signing secret is generated when you save. Every {label.toLowerCase()} issued before that
              point stops verifying immediately — anyone relying on one (an active session, an in-flight password
              reset link, ...) is signed out or has to start over.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                onRegenerateChange(true);
                setConfirming(false);
              }}
            >
              Regenerate on save
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
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

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <RuleField
          label="Sign-in rule"
          value={value.authRule}
          onChange={(authRule) => patch({ authRule })}
          publicOption={{ label: "Public", description: "Anyone matching the identity fields can attempt to authenticate." }}
          nullOption={{ label: "Admins", description: "Only superusers can authenticate as this collection." }}
        />
        <RuleField
          label="Manage rule"
          value={value.manageRule}
          onChange={(manageRule) => patch({ manageRule })}
          nullOption={{ label: "Admins", description: "Only superusers can list/impersonate/change other records here without their password." }}
          publicOption={{ label: "Public", description: "Anyone matching this rule can list, impersonate, or change other records here." }}
        />
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
        {value.oauth2Enabled ? (
          <div className="flex flex-col gap-2 rounded-lg border border-border bg-card p-3">
            <span className="text-xs font-medium text-foreground/80">Mapped fields</span>
            <p className="text-2xs leading-snug text-muted-foreground">
              Which field on this collection each provider's profile info is copied into on first sign-in. Leave a
              field blank to skip copying it.
            </p>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <OptionField label="External id field" help="Stores the provider's own user id. Leave blank to not record it.">
                <Input
                  value={value.oauth2MappedFields.id}
                  onChange={(e) => patch({ oauth2MappedFields: { ...value.oauth2MappedFields, id: e.target.value } })}
                  placeholder="e.g. externalId"
                  className="h-control-md font-mono text-sm"
                />
              </OptionField>
              <OptionField label="Name field">
                <Input
                  value={value.oauth2MappedFields.name}
                  onChange={(e) => patch({ oauth2MappedFields: { ...value.oauth2MappedFields, name: e.target.value } })}
                  placeholder="name"
                  className="h-control-md font-mono text-sm"
                />
              </OptionField>
              <OptionField label="Username field">
                <Input
                  value={value.oauth2MappedFields.username}
                  onChange={(e) => patch({ oauth2MappedFields: { ...value.oauth2MappedFields, username: e.target.value } })}
                  placeholder="username"
                  className="h-control-md font-mono text-sm"
                />
              </OptionField>
              <OptionField label="Avatar URL field">
                <Input
                  value={value.oauth2MappedFields.avatarURL}
                  onChange={(e) => patch({ oauth2MappedFields: { ...value.oauth2MappedFields, avatarURL: e.target.value } })}
                  placeholder="avatar"
                  className="h-control-md font-mono text-sm"
                />
              </OptionField>
            </div>
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

      <div className="flex flex-col gap-2 rounded-lg border border-border p-3">
        <ToggleField
          checked={value.magicLinkEnabled}
          onChange={(magicLinkEnabled) => patch({ magicLinkEnabled })}
          label="Magic link sign-in"
          help="Sign in with a single-use link emailed to the identity field, instead of a password or code."
        />
        {value.magicLinkEnabled ? (
          <>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <DurationField
                label="Link validity"
                value={value.magicLinkDuration}
                onChange={(magicLinkDuration) => patch({ magicLinkDuration })}
              />
              <OptionField
                label="URL template"
                help="Built when the request doesn't supply an allow-listed redirectUrl. Must include {TOKEN}."
              >
                <Input
                  value={value.magicLinkUrlTemplate}
                  onChange={(e) => patch({ magicLinkUrlTemplate: e.target.value })}
                  className="h-control-md font-mono text-sm"
                />
              </OptionField>
            </div>
            <EmailTemplateFields
              value={value.magicLinkTemplate}
              onChange={(magicLinkTemplate) => patch({ magicLinkTemplate })}
              placeholders="{APP_NAME}, {MAGIC_LINK}"
            />
          </>
        ) : null}
      </div>

      <div className="flex flex-col gap-2 rounded-lg border border-border p-3">
        <ToggleField
          checked={value.authAlertEnabled}
          onChange={(authAlertEnabled) => patch({ authAlertEnabled })}
          label="Login alerts"
          help="Email the identity field when a sign-in is detected from a new location."
        />
        {value.authAlertEnabled ? (
          <EmailTemplateFields
            value={value.authAlertTemplate}
            onChange={(authAlertTemplate) => patch({ authAlertTemplate })}
            placeholders="{APP_NAME}, {ALERT_INFO}"
          />
        ) : null}
      </div>

      <div className="flex flex-col gap-3">
        <div className="flex flex-col">
          <span className="text-sm font-medium text-foreground">Auth emails</span>
          <span className="text-xs text-muted-foreground">
            The verification, password-reset, and email-change-confirmation emails this collection sends. These are
            separate from the app-wide templates under Settings → Email → Templates, which cover everything sent
            through <code className="font-mono">POST /api/mails/send</code> and <code className="font-mono">
            _emailTriggers</code> instead.
          </span>
        </div>
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
          <div className="flex flex-col gap-1.5 rounded-lg border border-border p-3">
            <span className="text-xs font-medium text-foreground/80">Verification</span>
            <EmailTemplateFields
              value={value.verificationTemplate}
              onChange={(verificationTemplate) => patch({ verificationTemplate })}
              placeholders="{APP_NAME}, {APP_URL}, {TOKEN}"
            />
          </div>
          <div className="flex flex-col gap-1.5 rounded-lg border border-border p-3">
            <span className="text-xs font-medium text-foreground/80">Password reset</span>
            <EmailTemplateFields
              value={value.resetPasswordTemplate}
              onChange={(resetPasswordTemplate) => patch({ resetPasswordTemplate })}
              placeholders="{APP_NAME}, {APP_URL}, {TOKEN}"
            />
          </div>
          <div className="flex flex-col gap-1.5 rounded-lg border border-border p-3">
            <span className="text-xs font-medium text-foreground/80">Confirm email change</span>
            <EmailTemplateFields
              value={value.confirmEmailChangeTemplate}
              onChange={(confirmEmailChangeTemplate) => patch({ confirmEmailChangeTemplate })}
              placeholders="{APP_NAME}, {APP_URL}, {TOKEN}"
            />
          </div>
        </div>
      </div>

      <div className="flex flex-col gap-3">
        <div className="flex flex-col">
          <span className="text-sm font-medium text-foreground">Token durations</span>
          <span className="text-xs text-muted-foreground">
            How long each kind of token this collection issues stays valid. Shortening one doesn't affect tokens
            already handed out — it only changes what gets minted next. Regenerating a secret is immediate and
            invalidates every token of that kind already handed out.
          </span>
        </div>
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
          <TokenDurationField
            label="Auth token"
            duration={value.authTokenDuration}
            onDurationChange={(authTokenDuration) => patch({ authTokenDuration })}
            regenerate={value.regenerateAuthTokenSecret}
            onRegenerateChange={(regenerateAuthTokenSecret) => patch({ regenerateAuthTokenSecret })}
          />
          <TokenDurationField
            label="Password reset token"
            duration={value.passwordResetTokenDuration}
            onDurationChange={(passwordResetTokenDuration) => patch({ passwordResetTokenDuration })}
            regenerate={value.regeneratePasswordResetTokenSecret}
            onRegenerateChange={(regeneratePasswordResetTokenSecret) => patch({ regeneratePasswordResetTokenSecret })}
          />
          <TokenDurationField
            label="Email change token"
            duration={value.emailChangeTokenDuration}
            onDurationChange={(emailChangeTokenDuration) => patch({ emailChangeTokenDuration })}
            regenerate={value.regenerateEmailChangeTokenSecret}
            onRegenerateChange={(regenerateEmailChangeTokenSecret) => patch({ regenerateEmailChangeTokenSecret })}
          />
          <TokenDurationField
            label="Verification token"
            duration={value.verificationTokenDuration}
            onDurationChange={(verificationTokenDuration) => patch({ verificationTokenDuration })}
            regenerate={value.regenerateVerificationTokenSecret}
            onRegenerateChange={(regenerateVerificationTokenSecret) => patch({ regenerateVerificationTokenSecret })}
          />
          <TokenDurationField
            label="File token"
            duration={value.fileTokenDuration}
            onDurationChange={(fileTokenDuration) => patch({ fileTokenDuration })}
            regenerate={value.regenerateFileTokenSecret}
            onRegenerateChange={(regenerateFileTokenSecret) => patch({ regenerateFileTokenSecret })}
          />
        </div>
      </div>
    </section>
  );
}
