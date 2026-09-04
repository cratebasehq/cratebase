import type { CollectionModel, OAuth2Provider } from "pocketbase";
import { type FieldSchema, userFields } from "@/lib/field-types";

/** One OAuth2 provider, as this dashboard edits it. The SDK types `pkce`
 * as `boolean | undefined`; this form treats "unset" as a real third state
 * (auto-detect, PocketBase's own default) rather than collapsing it into
 * `false`, so it's re-typed as `boolean | null`. */
export type AuthProviderValue = Omit<OAuth2Provider, "logo" | "extra" | "pkce"> & { pkce: boolean | null };

/** `crates/core/src/collection.rs::AuthOptions`, flattened into the form's
 * own field names. Every duration is in seconds, matching the wire. Only
 * present when `type === "auth"` — a base collection has none of this. */
export interface AuthOptionsValue {
  passwordAuthEnabled: boolean;
  oauth2Enabled: boolean;
  oauth2Providers: AuthProviderValue[];
  mfaEnabled: boolean;
  mfaDuration: number;
  mfaRule: string;
  otpEnabled: boolean;
  otpDuration: number;
  otpLength: number;
  authTokenDuration: number;
  passwordResetTokenDuration: number;
  emailChangeTokenDuration: number;
  verificationTokenDuration: number;
  fileTokenDuration: number;
}

/** The editable shape of a collection, as the schema editor holds it —
 * everything `PATCH /api/collections/:id` accepts that this dashboard
 * exposes, and nothing else. */
export interface CollectionFormValue {
  name: string;
  type: "base" | "auth";
  identityField: string;
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
  };
}

/** Mirrors `cratebase_core::collection::AuthOptions::default()` — the
 * values a freshly created auth collection gets on the server, so a new
 * collection's form starts already agreeing with what create() will send. */
export function defaultAuthOptions(): AuthOptionsValue {
  return {
    passwordAuthEnabled: true,
    oauth2Enabled: false,
    oauth2Providers: [],
    mfaEnabled: false,
    mfaDuration: 600,
    mfaRule: "",
    otpEnabled: false,
    otpDuration: 180,
    otpLength: 8,
    authTokenDuration: 432_000,
    passwordResetTokenDuration: 1800,
    emailChangeTokenDuration: 1800,
    verificationTokenDuration: 86_400,
    fileTokenDuration: 180,
  };
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
      if (!provider.clientId.trim()) errors.push(`${label}: client id is required`);
      if (!provider.authURL.trim() || !provider.tokenURL.trim() || !provider.userInfoURL.trim()) {
        errors.push(`${label}: auth, token, and user-info URLs are required`);
      }
    });
  }
  if (auth.mfaEnabled && auth.mfaDuration < 1) errors.push("MFA duration must be at least 1 second");
  if (auth.otpEnabled && auth.otpDuration < 1) errors.push("OTP duration must be at least 1 second");
  if (auth.otpEnabled && auth.otpLength < 4) errors.push("OTP length must be at least 4 digits");
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
    identityField: "email",
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

export function collectionToFormValue(collection: CollectionModel): CollectionFormValue {
  return {
    name: collection.name,
    type: collection.type === "auth" ? "auth" : "base",
    identityField: (collection.type === "auth" ? collection.passwordAuth?.identityFields?.[0] : undefined) ?? "email",
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
            })),
            mfaEnabled: collection.mfa?.enabled ?? false,
            mfaDuration: collection.mfa?.duration ?? 600,
            mfaRule: collection.mfa?.rule ?? "",
            otpEnabled: collection.otp?.enabled ?? false,
            otpDuration: collection.otp?.duration ?? 180,
            otpLength: collection.otp?.length ?? 8,
            authTokenDuration: collection.authToken?.duration ?? 432_000,
            passwordResetTokenDuration: collection.passwordResetToken?.duration ?? 1800,
            emailChangeTokenDuration: collection.emailChangeToken?.duration ?? 1800,
            verificationTokenDuration: collection.verificationToken?.duration ?? 86_400,
            fileTokenDuration: collection.fileToken?.duration ?? 180,
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
export function authOptionsPayload(auth: AuthOptionsValue, identityField: string): Record<string, unknown> {
  return {
    // Both mutations previously sent `passwordAuth: { identityFields }`
    // on its own — folded in here so there's exactly one `passwordAuth`
    // key in the outgoing object. Two separate spreads of the same
    // top-level key would have the second silently clobber the first.
    passwordAuth: { enabled: auth.passwordAuthEnabled, identityFields: [identityField] },
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
        ...(p.pkce === null ? {} : { pkce: p.pkce }),
      })),
    },
    mfa: { enabled: auth.mfaEnabled, duration: auth.mfaDuration, rule: auth.mfaRule },
    otp: { enabled: auth.otpEnabled, duration: auth.otpDuration, length: auth.otpLength },
    authToken: { duration: auth.authTokenDuration },
    passwordResetToken: { duration: auth.passwordResetTokenDuration },
    emailChangeToken: { duration: auth.emailChangeTokenDuration },
    verificationToken: { duration: auth.verificationTokenDuration },
    fileToken: { duration: auth.fileTokenDuration },
  };
}
