/**
 * Schema-as-code for Cratebase. Builds the `{ "collections": [...] }` document
 * accepted by `POST /api/schema/apply`, `cratebase dev` (`./schema.json`) and
 * `cratebase schema`. Record types still come from `cratebase typegen`.
 */

export type CollectionType = "base" | "auth" | "view";

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
  | "file"
  | "relation"
  | "json"
  | "password"
  | "geoPoint"
  | "vector";

export interface CommonFieldOptions {
  required?: boolean;
  hidden?: boolean;
  presentable?: boolean;
  help?: string;
  /** Include in full-text search (text-shaped types only). */
  searchable?: boolean;
}

/** A serialized field: exactly the JSON shape Cratebase stores. */
export interface FieldDef extends CommonFieldOptions {
  name: string;
  type: FieldType;
  [key: string]: unknown;
}

const RELATION_TARGET = Symbol("cratebase.relationTarget");

type RelationField = FieldDef & { [RELATION_TARGET]?: string };

function field(
  name: string,
  type: FieldType,
  opts: object | undefined,
): FieldDef {
  const out: FieldDef = { name, type };
  if (opts) {
    for (const [k, v] of Object.entries(opts)) {
      if (v !== undefined) out[k] = v;
    }
  }
  return out;
}

export interface TextOptions extends CommonFieldOptions {
  min?: number;
  max?: number;
  pattern?: string;
  autogeneratePattern?: string;
  primaryKey?: boolean;
}
export const text = (name: string, o?: TextOptions): FieldDef => field(name, "text", o);

export interface EditorOptions extends CommonFieldOptions {
  maxSize?: number;
  convertURLs?: boolean;
}
export const editor = (name: string, o?: EditorOptions): FieldDef => field(name, "editor", o);

export interface NumberOptions extends CommonFieldOptions {
  min?: number;
  max?: number;
  onlyInt?: boolean;
}
export const number = (name: string, o?: NumberOptions): FieldDef => field(name, "number", o);

export const bool = (name: string, o?: CommonFieldOptions): FieldDef => field(name, "bool", o);

export interface DomainOptions extends CommonFieldOptions {
  exceptDomains?: string[];
  onlyDomains?: string[];
}
export const email = (name: string, o?: DomainOptions): FieldDef => field(name, "email", o);
export const url = (name: string, o?: DomainOptions): FieldDef => field(name, "url", o);

export interface DateOptions extends CommonFieldOptions {
  min?: string;
  max?: string;
}
export const date = (name: string, o?: DateOptions): FieldDef => field(name, "date", o);

export interface AutodateOptions extends CommonFieldOptions {
  onCreate?: boolean;
  onUpdate?: boolean;
}
export const autodate = (name: string, o?: AutodateOptions): FieldDef =>
  field(name, "autodate", o);

export interface SelectOptions extends CommonFieldOptions {
  maxSelect?: number;
}
export const select = (name: string, values: string[], o?: SelectOptions): FieldDef =>
  field(name, "select", { ...o, values });

export interface FileOptions extends CommonFieldOptions {
  maxSelect?: number;
  maxSize?: number;
  mimeTypes?: string[];
  thumbs?: string[];
  protected?: boolean;
}
export const file = (name: string, o?: FileOptions): FieldDef => field(name, "file", o);

export interface RelationOptions extends CommonFieldOptions {
  cascadeDelete?: boolean;
  minSelect?: number;
  maxSelect?: number;
}
/**
 * `collection` is the target collection's **name**; `defineSchema` resolves it
 * to the target's `collectionId`.
 */
export function relation(name: string, collection: string, o?: RelationOptions): FieldDef {
  const f = field(name, "relation", o) as RelationField;
  Object.defineProperty(f, RELATION_TARGET, { value: collection, enumerable: false });
  return f;
}

export const json = (name: string, o?: CommonFieldOptions & { maxSize?: number }): FieldDef =>
  field(name, "json", o);

export interface PasswordOptions extends CommonFieldOptions {
  min?: number;
  max?: number;
  pattern?: string;
  cost?: number;
}
export const password = (name: string, o?: PasswordOptions): FieldDef =>
  field(name, "password", o);

export const geoPoint = (name: string, o?: CommonFieldOptions): FieldDef =>
  field(name, "geoPoint", o);

export interface VectorOptions extends CommonFieldOptions {
  dimensions?: number;
  embedding?: { provider: string; model: string; sourceField: string };
}
export const vector = (name: string, o?: VectorOptions): FieldDef => field(name, "vector", o);

// ---- ids -------------------------------------------------------------------

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(input: string): number {
  const bytes = new TextEncoder().encode(input);
  let c = 0xffffffff;
  for (const b of bytes) c = CRC_TABLE[(c ^ b) & 0xff]! ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** Deterministic collection id: `pbc_<crc32(type + name)>` (matches the server). */
export function collectionId(type: CollectionType, name: string): string {
  return `pbc_${crc32(type + name)}`;
}

const BUILTIN_IDS: Record<string, string> = {
  users: "_pb_users_auth_",
  _superusers: collectionId("auth", "_superusers"),
};

// ---- indexes ---------------------------------------------------------------

/** Build a `CREATE [UNIQUE] INDEX` statement for a collection's `indexes[]`. */
export function idx(
  name: string,
  collection: string,
  columns: string[],
  opts: { unique?: boolean; where?: string } = {},
): string {
  const cols = columns.map((c) => `\`${c}\``).join(", ");
  const where = opts.where ? ` WHERE ${opts.where}` : "";
  return `CREATE ${opts.unique ? "UNIQUE " : ""}INDEX \`${name}\` ON \`${collection}\` (${cols})${where}`;
}

// ---- collections -----------------------------------------------------------

export interface CollectionInput {
  name: string;
  type?: CollectionType;
  fields: FieldDef[];
  indexes?: string[];
  /** `undefined`/`null` = superusers only, `""` = public, otherwise a filter. */
  listRule?: string | null;
  viewRule?: string | null;
  createRule?: string | null;
  updateRule?: string | null;
  deleteRule?: string | null;
  viewQuery?: string;
  searchLanguage?: string;
  ownerField?: string;
  /** Auth-collection options (`passwordAuth`, `oauth2`, `otp`, …), merged flat. */
  auth?: Record<string, unknown>;
}

export interface CollectionDef {
  id: string;
  name: string;
  type: CollectionType;
  fields: FieldDef[];
  indexes: string[];
  listRule: string | null;
  viewRule: string | null;
  createRule: string | null;
  updateRule: string | null;
  deleteRule: string | null;
  [key: string]: unknown;
}

export function defineCollection(input: CollectionInput): CollectionDef {
  const type = input.type ?? "base";
  const out: CollectionDef = {
    id: collectionId(type, input.name),
    name: input.name,
    type,
    fields: input.fields,
    indexes: input.indexes ?? [],
    listRule: input.listRule ?? null,
    viewRule: input.viewRule ?? null,
    createRule: input.createRule ?? null,
    updateRule: input.updateRule ?? null,
    deleteRule: input.deleteRule ?? null,
  };
  if (input.viewQuery !== undefined) out.viewQuery = input.viewQuery;
  if (input.searchLanguage !== undefined) out.searchLanguage = input.searchLanguage;
  if (input.ownerField !== undefined) out.ownerField = input.ownerField;
  if (input.auth) Object.assign(out, input.auth);
  return out;
}

export interface SchemaDocument {
  collections: CollectionDef[];
  toJSON(): { collections: CollectionDef[] };
}

export function defineSchema(...collections: CollectionDef[]): SchemaDocument {
  const ids = new Map<string, string>();
  for (const c of collections) {
    if (ids.has(c.name)) throw new Error(`duplicate collection "${c.name}" in schema`);
    ids.set(c.name, c.id);
  }
  const resolved = collections.map((c) => ({
    ...c,
    fields: c.fields.map((f) => {
      const target = (f as RelationField)[RELATION_TARGET];
      if (target === undefined) return f;
      const id = ids.get(target) ?? BUILTIN_IDS[target];
      if (id === undefined) {
        throw new Error(
          `relation "${f.name}": collection "${target}" is not defined in this schema (built-ins: users, _superusers)`,
        );
      }
      return { ...f, collectionId: id };
    }),
  }));
  return { collections: resolved, toJSON: () => ({ collections: resolved }) };
}

/** Write the schema as pretty JSON (e.g. `./schema.json` for `cratebase dev`). */
export async function writeSchema(schema: SchemaDocument, path: string | URL): Promise<void> {
  const { writeFile } = await import("node:fs/promises");
  await writeFile(path, JSON.stringify(schema, null, 2) + "\n");
}
