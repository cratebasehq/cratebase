import type { CollectionModel, OAuth2Provider } from "@cratebase/client";
import { type FieldSchema, userFields } from "@/lib/field-types";

/** One OAuth2 provider, as this dashboard edits it. The SDK types `pkce`
 * as `boolean | undefined`; this form treats "unset" as a real third state
 * (auto-detect, PocketBase's own default) rather than collapsing it into
 * `false`, so it's re-typed as `boolean | null`. `extra` carries whatever a
 * preset needs beyond client id/secret + the three URLs (Apple's
 * `teamId`/`keyId`/`privateKey`, Microsoft's `tenant`, GitLab's `baseUrl`,
 * a generic OIDC provider's `issuer`, or a `scope` override on any of
 * them) — narrowed to `Record<string, string>` here since every one of
 * those is a plain string on the wire too. */
export type AuthProviderValue = Omit<OAuth2Provider, "logo" | "extra" | "pkce"> & {
  pkce: boolean | null;
  extra: Record<string, string>;
};

/** One `subject`/`body` email template pair, embedded directly on the
 * collection (not the global `_emailTemplates` collection — these are
 * `cratebase_core::collection::EmailTemplate`, sent by the auth flow
 * itself, e.g. `POST /api/collections/:c/request-verification`). */
export interface AuthEmailTemplateValue {
  subject: string;
  body: string;
}

/** `crates/core/src/collection.rs::AuthOptions`, flattened into the form's
 * own field names. Every duration is in seconds, matching the wire. Only
 * present when `type === "auth"` — a base collection has none of this. */
export interface AuthOptionsValue {
  /** `null` = superusers only, `""` = public, anything else = a filter
   * expression — same three-state shape `RuleField` already renders for
   * the collection's list/view/create/update/delete rules. */
  authRule: string | null;
  manageRule: string | null;
  passwordAuthEnabled: boolean;
  oauth2Enabled: boolean;
  oauth2Providers: AuthProviderValue[];
  oauth2MappedFields: { id: string; name: string; username: string; avatarURL: string };
  mfaEnabled: boolean;
  mfaDuration: number;
  mfaRule: string;
  otpEnabled: boolean;
  otpDuration: number;
  otpLength: number;
  authAlertEnabled: boolean;
  authAlertTemplate: AuthEmailTemplateValue;
  magicLinkEnabled: boolean;
  magicLinkDuration: number;
  magicLinkUrlTemplate: string;
  magicLinkTemplate: AuthEmailTemplateValue;
  verificationTemplate: AuthEmailTemplateValue;
  resetPasswordTemplate: AuthEmailTemplateValue;
  confirmEmailChangeTemplate: AuthEmailTemplateValue;
  authTokenDuration: number;
  passwordResetTokenDuration: number;
  emailChangeTokenDuration: number;
  verificationTokenDuration: number;
  fileTokenDuration: number;
  /** Write-only: checking one of these and saving sends a freshly
   * generated random secret for that token kind, which invalidates every
   * token of that kind issued so far (its signature no longer matches).
   * The server never returns a stored secret, so there's nothing to show
   * here beyond "regenerate" — see `SecretSetting`'s doc comment for the
   * same pattern in app-wide settings. */
  regenerateAuthTokenSecret: boolean;
  regeneratePasswordResetTokenSecret: boolean;
  regenerateEmailChangeTokenSecret: boolean;
  regenerateVerificationTokenSecret: boolean;
  regenerateFileTokenSecret: boolean;
}

/** The editable shape of a collection, as the schema editor holds it —
 * everything `PATCH /api/collections/:id` accepts that this dashboard
 * exposes, and nothing else. */
export interface CollectionFormValue {
  name: string;
  type: "base" | "auth";
  /** `passwordAuth.identityFields` — one or more of `email`/`username`
   * that a caller may sign in with. Kept as an array all the way through
   * (rather than collapsing to one) so a collection can require both, or
   * offer either. */
  identityFields: string[];
  schema: FieldSchema[];
  /** Raw `CREATE INDEX` statements, exactly as the collection stores them. */
  indexes: string[];
  listRule: string | null;
  viewRule: string | null;
  createRule: string | null;
  updateRule: string | null;
  deleteRule: string | null;
  auth: AuthOptionsValue | null;
}

const NAME_RE = /^[a-zA-Z_][a-zA-Z0-9_]*$/;

/**
 * Mirrors `_collections.name TEXT NOT NULL UNIQUE` (case-sensitive,
 * SQLite's default BINARY collation — "Posts" and "posts" do NOT collide
 * server-side, so this check doesn't lowercase-normalize either). Without
 * it, a colliding name only surfaced as a raw "value for 'name' must be
 * unique" toast after the save round-trip instead of inline, right where
 * the name is typed.
 */
export function validateName(value: string, otherCollections: CollectionModel[]): string | null {
  if (value.length === 0) return "Name is required";
  if (!NAME_RE.test(value)) return "Letters, digits, underscore; can't start with a digit";
  if (otherCollections.some((c) => c.name === value)) {
    return "Another collection already uses this name";
  }
  return null;
}

// Mirrors `cratebase_core::field::RESERVED_FIELD_NAMES` — these columns
// are managed by the server and can't be redefined as schema fields.
const RESERVED_FIELD_NAMES = ["id", "created", "updated", "collectionId", "collectionName", "expand"];

/** Field name errors, shown inline next to the field's name input rather
 * than as a form-wide toast. Mirrors the constraints
 * `cratebase_core::field::is_valid_identifier` and `RESERVED_FIELD_NAMES`
 * enforce server-side, so a bad name is caught before the save round-trip. */
export function validateFieldName(field: FieldSchema, schema: FieldSchema[]): string | null {
  if (field.name.length === 0) return "Field name is required";
  if (field.name.length > 64) return "Field name must be 64 characters or fewer";
  if (!NAME_RE.test(field.name)) return "Letters, digits, underscore; can't start with a digit";
  if (RESERVED_FIELD_NAMES.includes(field.name)) return `"${field.name}" is a reserved field name`;
  if (schema.some((other) => other.id !== field.id && other.name === field.name)) {
    return "Another field already uses this name";
  }
  return null;
}

/** Type-specific option errors, shown inline inside the offending field's
 * options panel. Only checks constraints the API would otherwise reject on
 * save (a missing relation target, an empty select, an inverted min/max
 * range) — everything else is genuinely optional. */
export function validateFieldOptions(field: FieldSchema): string | null {
  if (field.type === "relation" && !field.collectionId) return "Choose a target collection";
  if (field.type === "select" && ((field.values as string[] | undefined)?.length ?? 0) === 0) {
    return "Add at least one option value";
  }
  if (field.type === "autodate" && !field.onCreate && !field.onUpdate) {
    return 'Enable "Set on create" or "Set on update"';
  }
  if (["text", "editor", "password", "number"].includes(field.type)) {
    const min = field.min as number | undefined;
    const max = field.max as number | undefined;
    if (typeof min === "number" && typeof max === "number" && min > max) {
      return field.type === "number" ? "Min value can't exceed max value" : "Min length can't exceed max length";
    }
  }
  return null;
}

/**
 * Everything wrong with the form right now, as one list.
 *
 * The old editor showed these inline but still let Save through, so a
 * schema with a nameless field or a relation pointing nowhere was sent to
 * the server and came back as a raw error toast. Save is gated on this
 * being empty.
 */
export function collectionFormErrors(value: CollectionFormValue, otherCollections: CollectionModel[]): string[] {
  const errors: string[] = [];
  const nameError = validateName(value.name, otherCollections);
  if (nameError) errors.push(`Name: ${nameError.toLowerCase()}`);
  for (const field of value.schema) {
    const named = validateFieldName(field, value.schema);
    if (named) errors.push(`${field.name || "Unnamed field"}: ${named.toLowerCase()}`);
    const options = validateFieldOptions(field);
    if (options) errors.push(`${field.name || "Unnamed field"}: ${options.toLowerCase()}`);
  }
  if (value.auth) errors.push(...validateAuthOptions(value.auth));
  return errors;
}

export function emptyOAuth2Provider(): AuthProviderValue {
  return {
    name: "",
    clientId: "",
    clientSecret: "",
    authURL: "",
    tokenURL: "",
    userInfoURL: "",
    displayName: "",
    pkce: null,
    extra: {},
  };
}

/** Presets whose Service ID/client id is authenticated with a per-request
 * JWT client secret (`extra.teamId`/`keyId`/`privateKey`) instead of a
 * static `clientSecret` — currently just Apple. */
export function isJwtClientSecretProvider(name: string): boolean {
  return name.trim().toLowerCase() === "apple";
}

/** Every preset `cratebase_auth::KnownProvider::from_name` recognizes —
 * its auth/token/userinfo URLs are baked in server-side from the name
 * alone, so this dashboard's own validation shouldn't demand them the
 * way it does for a hand-configured provider. */
export const KNOWN_PRESET_NAMES: ReadonlySet<string> = new Set([
  "google",
  "github",
  "apple",
  "microsoft",
  "discord",
  "gitlab",
  "facebook",
  "twitter",
  "linkedin",
  "slack",
  "twitch",
  "spotify",
]);

/** A provider configured with nothing but an issuer URL — auto-discovered
 * at login time (`GET {issuer}/.well-known/openid-configuration`), so it
 * needs none of the three endpoint URLs a hand-configured provider does.
 * Matches any name (conventionally `oidc`/`oidc2`/`oidc3`, PocketBase's own
 * convention for more than one), keyed on `extra.issuer` being set — same
 * rule `crates/server/src/routes/auth.rs`'s `oidc_issuer` uses. */
export function isGenericOidcProvider(extra: Record<string, string>): boolean {
  return Boolean(extra.issuer?.trim());
}

const DEFAULT_AUTH_ALERT_TEMPLATE: AuthEmailTemplateValue = {
  subject: "Login from a new location",
  body: "<p>Hello,</p>\n<p>We noticed a login to your {APP_NAME} account from a new location:</p>\n<p><em>{ALERT_INFO}</em></p>\n<p><strong>If this wasn't you, you should immediately change your {APP_NAME} account password to revoke access from all other locations.</strong></p>\n<p>If this was you, you may disregard this email.</p>\n<p>\n  Thanks,<br/>\n  {APP_NAME} team\n</p>",
};
const DEFAULT_MAGIC_LINK_TEMPLATE: AuthEmailTemplateValue = {
  subject: "Sign in to {APP_NAME}",
  body: '<p>Hello,</p>\n<p>Click on the button below to sign in to {APP_NAME}.</p>\n<p>\n  <a class="btn" href="{MAGIC_LINK}" target="_blank" rel="noopener">Sign in</a>\n</p>\n<p><i>If you didn\'t ask to sign in, you can ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {APP_NAME} team\n</p>',
};
const DEFAULT_VERIFICATION_TEMPLATE: AuthEmailTemplateValue = {
  subject: "Verify your {APP_NAME} email",
  body: '<p>Hello,</p>\n<p>Thank you for joining us at {APP_NAME}.</p>\n<p>Click on the button below to verify your email address.</p>\n<p>\n  <a class="btn" href="{APP_URL}/_/#/auth/confirm-verification/{TOKEN}" target="_blank" rel="noopener">Verify</a>\n</p>\n<p><i>If you didn\'t recently register, please ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {APP_NAME} team\n</p>',
};
const DEFAULT_RESET_PASSWORD_TEMPLATE: AuthEmailTemplateValue = {
  subject: "Reset your {APP_NAME} password",
  body: '<p>Hello,</p>\n<p>Click on the button below to reset your password.</p>\n<p>\n  <a class="btn" href="{APP_URL}/_/#/auth/confirm-password-reset/{TOKEN}" target="_blank" rel="noopener">Reset password</a>\n</p>\n<p><i>If you didn\'t ask to reset your password, please ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {APP_NAME} team\n</p>',
};
const DEFAULT_CONFIRM_EMAIL_CHANGE_TEMPLATE: AuthEmailTemplateValue = {
  subject: "Confirm your {APP_NAME} new email address",
  body: '<p>Hello,</p>\n<p>Click on the button below to confirm your new email address.</p>\n<p>\n  <a class="btn" href="{APP_URL}/_/#/auth/confirm-email-change/{TOKEN}" target="_blank" rel="noopener">Confirm new email</a>\n</p>\n<p><i>If you didn\'t ask to change your email address, please ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {APP_NAME} team\n</p>',
};

/** Mirrors `cratebase_core::collection::AuthOptions::default()` — the
 * values a freshly created auth collection gets on the server, so a new
 * collection's form starts already agreeing with what create() will send. */
export function defaultAuthOptions(): AuthOptionsValue {
  return {
    authRule: "",
    manageRule: null,
    passwordAuthEnabled: true,
    oauth2Enabled: false,
    oauth2Providers: [],
    oauth2MappedFields: { id: "", name: "name", username: "", avatarURL: "avatar" },
    mfaEnabled: false,
    mfaDuration: 600,
    mfaRule: "",
    otpEnabled: false,
    otpDuration: 180,
    otpLength: 8,
    authAlertEnabled: true,
    authAlertTemplate: DEFAULT_AUTH_ALERT_TEMPLATE,
    magicLinkEnabled: false,
    magicLinkDuration: 900,
    magicLinkUrlTemplate: "{APP_URL}/auth/magic-link?token={TOKEN}",
    magicLinkTemplate: DEFAULT_MAGIC_LINK_TEMPLATE,
    verificationTemplate: DEFAULT_VERIFICATION_TEMPLATE,
    resetPasswordTemplate: DEFAULT_RESET_PASSWORD_TEMPLATE,
    confirmEmailChangeTemplate: DEFAULT_CONFIRM_EMAIL_CHANGE_TEMPLATE,
    authTokenDuration: 432_000,
    passwordResetTokenDuration: 1800,
    emailChangeTokenDuration: 1800,
    verificationTokenDuration: 86_400,
    fileTokenDuration: 180,
    regenerateAuthTokenSecret: false,
    regeneratePasswordResetTokenSecret: false,
    regenerateEmailChangeTokenSecret: false,
    regenerateVerificationTokenSecret: false,
    regenerateFileTokenSecret: false,
  };
}

/** A random, URL-safe token-signing secret suffix, generated client-side
 * only when "regenerate" is checked — the server never hands one back, so
 * there's nothing to read and re-send, only a fresh one to mint. */
export function randomTokenSecret(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(32));
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** Options errors, folded into the same list `collectionFormErrors`
 * returns. A provider needs a name (it keys the OAuth2 login endpoint) and
 * the three URLs every non-preset provider requires; PocketBase's own
 * dashboard enforces the same three before letting a custom provider save. */
export function validateAuthOptions(auth: AuthOptionsValue): string[] {
  const errors: string[] = [];
  if (auth.oauth2Enabled) {
    auth.oauth2Providers.forEach((provider, i) => {
      const label = provider.name || `Provider ${i + 1}`;
      if (!provider.name.trim()) errors.push(`OAuth2 provider ${i + 1} needs a name`);
      if (!provider.clientId.trim()) errors.push(`${label}: client id is required (Apple's Service ID)`);

      if (isJwtClientSecretProvider(provider.name)) {
        // Apple: no static clientSecret at all -- it's minted per
        // request from these three instead (see
        // `cratebase_auth::apple_client_secret`).
        if (!provider.extra.teamId?.trim()) errors.push(`${label}: Team ID is required`);
        if (!provider.extra.keyId?.trim()) errors.push(`${label}: Key ID is required`);
        if (!provider.extra.privateKey?.trim()) errors.push(`${label}: private key is required`);
      } else if (!isGenericOidcProvider(provider.extra)) {
        // A generic OIDC provider is discovered at login time from just
        // its issuer, so it's the one case (besides Apple) that skips
        // this -- every other preset and hand-configured provider still
        // needs its own auth/token/userinfo URLs (a preset's are filled
        // in server-side from the name alone, so leaving them blank
        // here is fine and expected).
        if (!provider.authURL.trim() || !provider.tokenURL.trim() || !provider.userInfoURL.trim()) {
          if (!KNOWN_PRESET_NAMES.has(provider.name.trim().toLowerCase())) {
            errors.push(`${label}: auth, token, and user-info URLs are required`);
          }
        }
      }
    });
  }
  if (auth.mfaEnabled && auth.mfaDuration < 1) errors.push("MFA duration must be at least 1 second");
  if (auth.otpEnabled && auth.otpDuration < 1) errors.push("OTP duration must be at least 1 second");
  if (auth.otpEnabled && auth.otpLength < 4) errors.push("OTP length must be at least 4 digits");
  if (auth.magicLinkEnabled && auth.magicLinkDuration < 1) {
    errors.push("Magic link duration must be at least 1 second");
  }
  if (auth.magicLinkEnabled && !auth.magicLinkUrlTemplate.trim()) {
    errors.push("Magic link URL template is required while magic links are enabled");
  }
  if (auth.magicLinkEnabled && auth.magicLinkUrlTemplate.trim() && !auth.magicLinkUrlTemplate.includes("{TOKEN}")) {
    errors.push("Magic link URL template must include {TOKEN}");
  }
  for (const [label, duration] of [
    ["Auth token", auth.authTokenDuration],
    ["Password reset token", auth.passwordResetTokenDuration],
    ["Email change token", auth.emailChangeTokenDuration],
    ["Verification token", auth.verificationTokenDuration],
    ["File token", auth.fileTokenDuration],
  ] as const) {
    if (duration < 1) errors.push(`${label} duration must be at least 1 second`);
  }
  return errors;
}

export function emptyCollectionForm(type: "base" | "auth" = "base"): CollectionFormValue {
  return {
    name: "",
    type,
    identityFields: ["email"],
    schema: [],
    indexes: [],
    listRule: null,
    viewRule: null,
    createRule: null,
    updateRule: null,
    deleteRule: null,
    auth: type === "auth" ? defaultAuthOptions() : null,
  };
}

function templateOf(t: { subject: string; body: string } | undefined, fallback: AuthEmailTemplateValue): AuthEmailTemplateValue {
  return t ? { subject: t.subject, body: t.body } : fallback;
}

export function collectionToFormValue(collection: CollectionModel): CollectionFormValue {
  const identityFields =
    collection.type === "auth" && collection.passwordAuth?.identityFields?.length
      ? [...collection.passwordAuth.identityFields]
      : ["email"];
  return {
    name: collection.name,
    type: collection.type === "auth" ? "auth" : "base",
    identityFields,
    schema: userFields(collection),
    indexes: [...(collection.indexes ?? [])],
    listRule: collection.listRule ?? null,
    viewRule: collection.viewRule ?? null,
    createRule: collection.createRule ?? null,
    updateRule: collection.updateRule ?? null,
    deleteRule: collection.deleteRule ?? null,
    auth:
      collection.type === "auth"
        ? {
            authRule: collection.authRule ?? "",
            manageRule: collection.manageRule ?? null,
            passwordAuthEnabled: collection.passwordAuth?.enabled ?? true,
            oauth2Enabled: collection.oauth2?.enabled ?? false,
            oauth2Providers: (collection.oauth2?.providers ?? []).map((p) => ({
              name: p.name,
              clientId: p.clientId,
              // Never sent back by the server (`#[serde(skip_serializing)]`
              // on `OAuth2Provider::client_secret`) — the form always
              // starts blank for an existing provider.
              clientSecret: "",
              authURL: p.authURL,
              tokenURL: p.tokenURL,
              userInfoURL: p.userInfoURL,
              displayName: p.displayName,
              pkce: p.pkce ?? null,
              extra: Object.fromEntries(
                Object.entries(p.extra ?? {}).map(([k, v]) => [k, v == null ? "" : String(v)]),
              ),
            })),
            oauth2MappedFields: {
              id: collection.oauth2?.mappedFields?.id ?? "",
              name: collection.oauth2?.mappedFields?.name ?? "name",
              username: collection.oauth2?.mappedFields?.username ?? "",
              avatarURL: collection.oauth2?.mappedFields?.avatarURL ?? "avatar",
            },
            mfaEnabled: collection.mfa?.enabled ?? false,
            mfaDuration: collection.mfa?.duration ?? 600,
            mfaRule: collection.mfa?.rule ?? "",
            otpEnabled: collection.otp?.enabled ?? false,
            otpDuration: collection.otp?.duration ?? 180,
            otpLength: collection.otp?.length ?? 8,
            authAlertEnabled: collection.authAlert?.enabled ?? true,
            authAlertTemplate: templateOf(collection.authAlert?.emailTemplate, DEFAULT_AUTH_ALERT_TEMPLATE),
            magicLinkEnabled: collection.magicLink?.enabled ?? false,
            magicLinkDuration: collection.magicLink?.duration ?? 900,
            magicLinkUrlTemplate: collection.magicLink?.urlTemplate ?? "{APP_URL}/auth/magic-link?token={TOKEN}",
            magicLinkTemplate: templateOf(collection.magicLink?.emailTemplate, DEFAULT_MAGIC_LINK_TEMPLATE),
            verificationTemplate: templateOf(collection.verificationTemplate, DEFAULT_VERIFICATION_TEMPLATE),
            resetPasswordTemplate: templateOf(collection.resetPasswordTemplate, DEFAULT_RESET_PASSWORD_TEMPLATE),
            confirmEmailChangeTemplate: templateOf(
              collection.confirmEmailChangeTemplate,
              DEFAULT_CONFIRM_EMAIL_CHANGE_TEMPLATE,
            ),
            authTokenDuration: collection.authToken?.duration ?? 432_000,
            passwordResetTokenDuration: collection.passwordResetToken?.duration ?? 1800,
            emailChangeTokenDuration: collection.emailChangeToken?.duration ?? 1800,
            verificationTokenDuration: collection.verificationToken?.duration ?? 86_400,
            fileTokenDuration: collection.fileToken?.duration ?? 180,
            regenerateAuthTokenSecret: false,
            regeneratePasswordResetTokenSecret: false,
            regenerateEmailChangeTokenSecret: false,
            regenerateVerificationTokenSecret: false,
            regenerateFileTokenSecret: false,
          }
        : null,
  };
}
/**
 * `AuthOptionsValue`, back to the flat wire keys `PATCH`/`POST
 * /api/collections` expect. Both the create and update mutations send the
 * whole thing whenever `type === "auth"` — `crates/server/src/routes/
 * collections.rs::update` merges only whole top-level keys, so a partial
 * `oauth2` here would still replace the *entire* stored `oauth2` object,
 * silently dropping every field this form doesn't know about (there are
 * none today, but the intent is the same reason `passwordAuth` is already
 * resent in full above this function).
 */
export function authOptionsPayload(auth: AuthOptionsValue, identityFields: string[]): Record<string, unknown> {
  function tokenPayload(duration: number, regenerate: boolean): Record<string, unknown> {
    return regenerate ? { duration, secret: randomTokenSecret() } : { duration };
  }
  return {
    authRule: auth.authRule,
    manageRule: auth.manageRule,
    // Both mutations previously sent `passwordAuth: { identityFields }`
    // on its own — folded in here so there's exactly one `passwordAuth`
    // key in the outgoing object. Two separate spreads of the same
    // top-level key would have the second silently clobber the first.
    passwordAuth: { enabled: auth.passwordAuthEnabled, identityFields },
    oauth2: {
      enabled: auth.oauth2Enabled,
      providers: auth.oauth2Providers.map((p) => ({
        name: p.name,
        clientId: p.clientId,
        clientSecret: p.clientSecret,
        authURL: p.authURL,
        tokenURL: p.tokenURL,
        userInfoURL: p.userInfoURL,
        displayName: p.displayName,
        extra: p.extra,
        ...(p.pkce === null ? {} : { pkce: p.pkce }),
      })),
      mappedFields: auth.oauth2MappedFields,
    },
    mfa: { enabled: auth.mfaEnabled, duration: auth.mfaDuration, rule: auth.mfaRule },
    otp: { enabled: auth.otpEnabled, duration: auth.otpDuration, length: auth.otpLength },
    authAlert: { enabled: auth.authAlertEnabled, emailTemplate: auth.authAlertTemplate },
    magicLink: {
      enabled: auth.magicLinkEnabled,
      duration: auth.magicLinkDuration,
      urlTemplate: auth.magicLinkUrlTemplate,
      emailTemplate: auth.magicLinkTemplate,
    },
    verificationTemplate: auth.verificationTemplate,
    resetPasswordTemplate: auth.resetPasswordTemplate,
    confirmEmailChangeTemplate: auth.confirmEmailChangeTemplate,
    authToken: tokenPayload(auth.authTokenDuration, auth.regenerateAuthTokenSecret),
    passwordResetToken: tokenPayload(auth.passwordResetTokenDuration, auth.regeneratePasswordResetTokenSecret),
    emailChangeToken: tokenPayload(auth.emailChangeTokenDuration, auth.regenerateEmailChangeTokenSecret),
    verificationToken: tokenPayload(auth.verificationTokenDuration, auth.regenerateVerificationTokenSecret),
    fileToken: tokenPayload(auth.fileTokenDuration, auth.regenerateFileTokenSecret),
  };
}
