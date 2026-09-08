/** Shapes shared across every module — the wire contract, not any one
 * endpoint's request/response. */

/** Every record Cratebase returns, auth or not. */
export interface RecordModel {
  id: string;
  collectionId: string;
  collectionName: string;
  created?: string;
  updated?: string;
  [key: string]: unknown;
}

/** One field of a `CollectionModel`, as `GET /api/collections` returns
 * it. Field-kind-specific options live in `options` untyped — the
 * dashboard and schema tooling are the only callers that need them. */
export interface CollectionField {
  id: string;
  name: string;
  type: string;
  system?: boolean;
  required?: boolean;
  hidden?: boolean;
  presentable?: boolean;
  [key: string]: unknown;
}

/** One `subject`/`body` HTML email template
 * (`crates/core/src/collection.rs`'s `EmailTemplate`). */
export interface EmailTemplate {
  subject: string;
  body: string;
}

/** A signing/expiry policy for one token kind
 * (`crates/core/src/collection.rs`'s `TokenConfig`). */
export interface TokenConfig {
  duration: number;
  secret?: string;
}

export interface AuthAlert {
  enabled: boolean;
  emailTemplate: EmailTemplate;
}

export interface OAuth2MappedFields {
  id: string;
  name: string;
  username: string;
  avatarURL: string;
}

export interface OAuth2Config {
  enabled: boolean;
  providers: OAuth2Provider[];
  mappedFields: OAuth2MappedFields;
}

export interface PasswordAuth {
  enabled: boolean;
  identityFields: string[];
}

export interface MfaConfig {
  enabled: boolean;
  duration: number;
  rule: string;
}

export interface OtpConfig {
  enabled: boolean;
  duration: number;
  length: number;
  emailTemplate: EmailTemplate;
}

/** A collection's own schema, as `GET /api/collections` returns it. Every
 * `auth`-only field is present (with Cratebase's own defaults) only when
 * `type === "auth"` — flattened onto the collection JSON exactly as
 * `crates/core/src/collection.rs`'s `AuthOptions` serializes, not nested
 * under an `auth` key. */
export interface CollectionModel {
  id: string;
  name: string;
  type: "base" | "auth" | "view";
  system?: boolean;
  fields: CollectionField[];
  indexes?: string[];
  listRule?: string | null;
  viewRule?: string | null;
  createRule?: string | null;
  updateRule?: string | null;
  deleteRule?: string | null;
  created?: string;
  updated?: string;
  /** View collections only. */
  viewQuery?: string;
  /** Auth collections only, below. `None` = superusers only. */
  authRule?: string | null;
  manageRule?: string | null;
  authAlert?: AuthAlert;
  oauth2?: OAuth2Config;
  passwordAuth?: PasswordAuth;
  mfa?: MfaConfig;
  otp?: OtpConfig;
  authToken?: TokenConfig;
  passwordResetToken?: TokenConfig;
  emailChangeToken?: TokenConfig;
  verificationToken?: TokenConfig;
  fileToken?: TokenConfig;
  verificationTemplate?: EmailTemplate;
  resetPasswordTemplate?: EmailTemplate;
  confirmEmailChangeTemplate?: EmailTemplate;
  [key: string]: unknown;
}

/** A field as a `POST`/`PATCH /api/collections` request sends it — `id` is
 * optional (the server generates one for a genuinely new field; omitting
 * it on an existing field's entry is how the dashboard's schema editor
 * would *add* one, not identify one) unlike {@link CollectionField}, which
 * is the read shape where the server always includes it. */
export type CollectionFieldInput = Partial<Omit<CollectionField, "id" | "name" | "type">> & {
  id?: string;
  name: string;
  type: string;
};

/** A collection as a `POST`/`PATCH /api/collections` request sends it —
 * every field of {@link CollectionModel} stays optional (a `PATCH` sends
 * only what changes), but `fields`, when present, takes
 * {@link CollectionFieldInput} rather than the read shape. */
export type CollectionInput = Partial<Omit<CollectionModel, "fields">> & {
  fields?: CollectionFieldInput[];
};

/** One configured OAuth2 provider entry on a collection's `auth.oauth2.providers`
 * (`crates/core/src/collection.rs`'s `OAuth2Provider`). `pkce` is `boolean |
 * undefined` on the wire — unset means "auto-detect" (the server's own
 * default), not `false`. No index signature: dashboard code `Omit`s
 * fields off this type (`AuthProviderValue` in `collection-form-value.ts`),
 * and a trailing `[key: string]: unknown` would make `keyof` collapse to
 * `string`, losing every named field's specific type through the `Omit`. */
export interface OAuth2Provider {
  name: string;
  clientId: string;
  clientSecret: string;
  authURL: string;
  tokenURL: string;
  userInfoURL: string;
  displayName: string;
  pkce?: boolean;
  extra?: Record<string, unknown>;
}

/** `GET .../records`'s envelope. `totalItems`/`totalPages` are real
 * numbers even under `skipTotal` — the server returns `-1` for both
 * rather than omitting the keys (`crates/server/src/routes/records.rs`'s
 * `Page` has non-optional `i64` fields). */
export interface ListResult<T> {
  page: number;
  perPage: number;
  totalItems: number;
  totalPages: number;
  items: T[];
}

export type SortSpec<T> = `${"" | "-" | "+"}${Extract<keyof T, string>}` | (string & {});

/** Query options for `list`/`fullList`, mapping 1:1 onto
 * `ListQuery` (`crates/server/src/routes/records.rs`). */
export interface ListOptions<T = RecordModel> {
  page?: number;
  perPage?: number;
  sort?: SortSpec<T>;
  filter?: string;
  expand?: string;
  fields?: string;
  skipTotal?: boolean;
  signal?: AbortSignal;
}

export interface ViewOptions {
  expand?: string;
  fields?: string;
  signal?: AbortSignal;
}

export interface WriteOptions {
  expand?: string;
  fields?: string;
  signal?: AbortSignal;
}

/** A realtime event for one record, as `GET /api/realtime` streams it. */
export interface RecordSubscription<T = RecordModel> {
  action: "create" | "update" | "delete";
  record: T;
}

export interface SubscribeOptions {
  filter?: string;
  fields?: string;
  expand?: string;
}
