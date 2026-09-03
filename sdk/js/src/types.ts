/** Every record carries these regardless of its collection's schema. */
export interface RecordModel {
  id: string;
  created: string;
  updated: string;
  collectionId: string;
  collectionName: string;
  [key: string]: unknown;
}

/** A record from an `auth`-typed collection additionally exposes `email`
 * and `verified`. */
export interface AuthRecord extends RecordModel {
  email?: string;
  verified?: boolean;
}

export interface AdminModel {
  id: string;
  email: string;
  created: string;
  updated: string;
  [key: string]: unknown;
}

export interface ListResult<T> {
  page: number;
  perPage: number;
  totalItems: number;
  totalPages: number;
  items: T[];
}

export type FieldType =
  | "text"
  | "editor"
  | "number"
  | "bool"
  | "email"
  | "url"
  | "date"
  | "autodate"
  | "select"
  | "json"
  | "relation"
  | "file"
  | "password";

export interface FieldSchema {
  id: string;
  name: string;
  type: FieldType;
  required?: boolean;
  unique?: boolean;
  options?: Record<string, unknown>;
}

export type CollectionType = "base" | "auth" | "view";

export interface AuthOptions {
  minPasswordLength?: number;
  /** Which schema field identifies an auth record for login. Defaults to
   * `"email"`; set to `"username"` (or any other field name) to log in
   * with something other than an email address. */
  identityField?: string;
  requireEmailVerification?: boolean;
  tokenTtlSeconds?: number;
}

export interface CollectionModel {
  id: string;
  name: string;
  type: CollectionType;
  schema: FieldSchema[];
  listRule: string | null;
  viewRule: string | null;
  createRule: string | null;
  updateRule: string | null;
  deleteRule: string | null;
  authOptions?: AuthOptions;
  created: string;
  updated: string;
}

export interface ListOptions {
  filter?: string;
  sort?: string;
}

export interface AuthResponse<T> {
  token: string;
  record: T;
}

export interface AdminAuthResponse {
  token: string;
  admin: AdminModel;
}

export interface OAuth2ProviderInfo {
  name: string;
  authUrl: string;
}

export interface AuthMethodsResponse {
  password: boolean;
  oauth2: {
    enabled: boolean;
    providers: OAuth2ProviderInfo[];
  };
}
