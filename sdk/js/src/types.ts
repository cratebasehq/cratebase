/** Every record carries these regardless of its collection's schema. */
export interface RecordModel {
  id: string;
  created: string;
  updated: string;
  collectionId: string;
  collectionName: string;
  [key: string]: unknown;
}

/** A record from an `auth`-typed collection additionally exposes `email`. */
export interface AuthRecord extends RecordModel {
  email?: string;
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
  authOptions?: Record<string, unknown>;
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
