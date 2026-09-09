import type { CollectionModel } from "@cratebase/client";
import { userFields, type FieldSchema } from "@/lib/field-types";

/**
 * Per-collection REST reference, built from the collection's own schema and
 * rules rather than a hand-maintained doc page. `openapi.yaml` at the repo
 * root is the source of truth for *which* endpoints exist, their paths, and
 * their envelope shapes (`{page, items: [...]}`, `{token, record}`, …) — the
 * constants below were read off it. It is deliberately not fetched or
 * parsed at runtime: it has no notion of *this* collection's fields, so it
 * cannot produce a `title`/`email`/`ownerId`-shaped example body or an
 * accurate rule description — only the live `CollectionModel` can. Treat
 * this file as `openapi.yaml`'s endpoint list, retargeted per collection.
 */

export type Method = "GET" | "POST" | "PATCH" | "DELETE";

/** Mirrors PocketBase's own phrasing for the four rule states a collection
 * can be in, so "null means superusers only" doesn't have to be relearned
 * per rule field. */
export type RuleTone = "public" | "restricted" | "locked" | "fixed";

export interface RuleInfo {
  /** Which rule field this is, e.g. `listRule` — omitted for endpoints no
   * rule governs (auth-with-password, auth-methods). */
  field?: string;
  value: string | null | undefined;
  tone: RuleTone;
  /** Human sentence, PocketBase-dashboard style. */
  summary: string;
}

export interface DocEndpoint {
  id: string;
  label: string;
  method: Method;
  /** Path with the real collection id/name substituted; `:id` is a literal
   * placeholder for a record id, matched by `openapi.yaml`'s `{id}`. */
  path: string;
  rule: RuleInfo;
  headers: { name: string; value: string; required: boolean }[];
  requestBody?: unknown;
  responseStatus: number;
  responseBody?: unknown;
  curl: string;
  js: string;
}

/** A plausible request-time value for a field, keyed off its type — the
 * same type switch every field renderer in this dashboard already keys
 * off of (see `schema-field-row.tsx`), reused here instead of re-deriving
 * per-type example logic a second time. */
export function sampleRequestValue(field: FieldSchema): unknown {
  const multi = Number(field.maxSelect ?? 1) > 1;
  switch (field.type) {
    case "number":
      return 42;
    case "bool":
      return true;
    case "json":
      return { note: "anything JSON-serialisable" };
    case "date":
      return "2026-01-31 12:00:00.000Z";
    case "email":
      return "someone@example.com";
    case "url":
      return "https://example.com";
    case "editor":
      return "<p>Some rich text.</p>";
    case "geoPoint":
      return { lon: -122.4194, lat: 37.7749 };
    case "vector":
      return [0.12, -0.34, 0.56];
    case "select": {
      const values = (field.values as string[] | undefined) ?? [];
      const first = values[0] ?? "value";
      return multi ? [first] : first;
    }
    case "relation":
      return multi ? ["RELATED_RECORD_ID"] : "RELATED_RECORD_ID";
    case "file":
      return multi ? "(files — send as multipart/form-data)" : "(file — send as multipart/form-data)";
    case "password":
      return "a-password";
    default:
      return `Example ${field.name}`;
  }
}

/** The server-side echo of the same field once it's landed in a record —
 * relations and selects round-trip as-is, but a `file` field answers with
 * the stored filename(s) rather than the upload it was sent as. */
function sampleResponseValue(field: FieldSchema): unknown {
  if (field.type === "file") {
    const multi = Number(field.maxSelect ?? 1) > 1;
    return multi
      ? [`${field.name}_9f8a2b3c.png`, `${field.name}_4d5e6f7a.png`]
      : `${field.name}_9f8a2b3c.png`;
  }
  return sampleRequestValue(field);
}

/** PocketBase's own dashboard puts it exactly this bluntly: a `null` rule
 * is superuser-only, `""` is public, anything else is a filter expression
 * evaluated against the request. A whitespace-only rule (e.g. the rule
 * editor's own Custom-mode placeholder, `" "`) is neither — `evaluate()`
 * in `crates/db/src/rules.rs` treats any rule whose `.trim()` is empty as
 * `AllowAll`, so it must be documented as public too, not "restricted". */
function describeRule(field: string, value: string | null | undefined): RuleInfo {
  const trimmed = value?.trim();
  if (value === null) {
    return {
      field,
      value,
      tone: "locked",
      summary: "Only superusers can perform this action.",
    };
  }
  if (trimmed === "") {
    return {
      field,
      value,
      tone: "public",
      summary: "Everyone (including guests) can perform this action.",
    };
  }
  return {
    field,
    value,
    tone: "restricted",
    summary: `Only requests matching this rule can perform this action: ${value}`,
  };
}

const AUTH_HEADER_REQUIRED = { name: "Authorization", value: "Bearer YOUR_TOKEN", required: true };
const CONTENT_TYPE_JSON = { name: "Content-Type", value: "application/json", required: true };

function headersFor(rule: RuleInfo, extra: { name: string; value: string; required: boolean }[] = []) {
  const headers = [...extra];
  // A locked (superuser-only) rule always needs auth. A restricted custom
  // rule only needs it if the rule text actually inspects the caller's
  // identity — plenty of custom rules (e.g. `published = true`) are
  // satisfiable by anonymous callers and shouldn't claim otherwise.
  const restrictedNeedsAuth = rule.tone === "restricted" && rule.value?.includes("@request.auth");
  if (rule.tone === "locked" || restrictedNeedsAuth) headers.push(AUTH_HEADER_REQUIRED);
  return headers;
}

function curlFor(method: Method, url: string, headers: { name: string; value: string }[], body?: unknown): string {
  const lines = [`curl -X ${method} "${url}"`];
  for (const h of headers) lines.push(`  -H "${h.name}: ${h.value}"`);
  if (body !== undefined) lines.push(`  -d '${JSON.stringify(body, null, 2)}'`);
  return lines.join(" \\\n");
}

function sampleRecord(collection: CollectionModel, fields: FieldSchema[], identityField: string): Record<string, unknown> {
  const record: Record<string, unknown> = { id: "RECORD_ID" };
  if (collection.type === "auth") {
    record[identityField] = identityField === "email" ? "someone@example.com" : "someuser";
    record.verified = true;
    record.emailVisibility = false;
  }
  for (const field of fields) record[field.name] = sampleResponseValue(field);
  // View collections are read-only projections with no backing table of
  // their own — `crates/db` never stamps them with `created`/`updated`
  // columns, so a docs example that invents them would be wrong.
  if (collection.type !== "view") {
    record.created = "2026-01-31 12:00:00.000Z";
    record.updated = "2026-01-31 12:00:00.000Z";
  }
  record.collectionId = collection.id;
  record.collectionName = collection.name;
  return record;
}

/** Every REST endpoint this collection exposes, in the order PocketBase's
 * own API-preview lists them: auth actions first for auth collections, then
 * the five CRUD verbs. Each endpoint carries everything the docs tab needs
 * to render one panel — no further lookups against `collection` required. */
export function buildDocEndpoints(collection: CollectionModel, origin: string): DocEndpoint[] {
  const name = collection.name;
  const fields = userFields(collection);
  const base = `${origin}/api/collections/${name}`;
  const identityField = collection.type === "auth" ? (collection.passwordAuth?.identityFields?.[0] ?? "email") : "email";

  const requestBody = Object.fromEntries(fields.map((f) => [f.name, sampleRequestValue(f)]));
  // Auth collections require email/password/passwordConfirm on create
  // (`password_field`/`email_field` are mandatory and must match — see
  // `crates/server/src/routes/records.rs`'s create_record) but
  // `userFields()` strips every system field, so the plain requestBody
  // above is missing them; the update example doesn't need them since a
  // PATCH is optional-password.
  const createRequestBody =
    collection.type === "auth"
      ? { email: "someone@example.com", password: "a-password", passwordConfirm: "a-password", ...requestBody }
      : requestBody;
  const record = sampleRecord(collection, fields, identityField);

  const listRule = describeRule("listRule", collection.listRule);
  const viewRule = describeRule("viewRule", collection.viewRule);
  const createRule = describeRule("createRule", collection.createRule);
  const updateRule = describeRule("updateRule", collection.updateRule);
  const deleteRule = describeRule("deleteRule", collection.deleteRule);

  const list: DocEndpoint = {
    id: "list",
    label: "List / search records",
    method: "GET",
    path: `/api/collections/${name}/records`,
    rule: listRule,
    headers: headersFor(listRule),
    responseStatus: 200,
    responseBody: { page: 1, perPage: 30, totalItems: 1, totalPages: 1, items: [record] },
    curl: curlFor("GET", `${base}/records?perPage=30&sort=-created`, headersFor(listRule)),
    js: `const result = await cb.collection("${name}").list({
  page: 1,
  perPage: 30,
  sort: "-created",
});`,
  };

  const view: DocEndpoint = {
    id: "view",
    label: "View record",
    method: "GET",
    path: `/api/collections/${name}/records/:id`,
    rule: viewRule,
    headers: headersFor(viewRule),
    responseStatus: 200,
    responseBody: record,
    curl: curlFor("GET", `${base}/records/RECORD_ID`, headersFor(viewRule)),
    js: `const record = await cb.collection("${name}").one("RECORD_ID");`,
  };

  const create: DocEndpoint = {
    id: "create",
    label: "Create record",
    method: "POST",
    path: `/api/collections/${name}/records`,
    rule: createRule,
    headers: headersFor(createRule, [CONTENT_TYPE_JSON]),
    requestBody: createRequestBody,
    responseStatus: 200,
    responseBody: record,
    curl: curlFor("POST", `${base}/records`, headersFor(createRule, [CONTENT_TYPE_JSON]), createRequestBody),
    js: `const record = await cb.collection("${name}").create(${JSON.stringify(createRequestBody, null, 2)});`,
  };

  const update: DocEndpoint = {
    id: "update",
    label: "Update record",
    method: "PATCH",
    path: `/api/collections/${name}/records/:id`,
    rule: updateRule,
    headers: headersFor(updateRule, [CONTENT_TYPE_JSON]),
    requestBody,
    responseStatus: 200,
    responseBody: record,
    curl: curlFor("PATCH", `${base}/records/RECORD_ID`, headersFor(updateRule, [CONTENT_TYPE_JSON]), requestBody),
    js: `const record = await cb.collection("${name}").update("RECORD_ID", ${JSON.stringify(requestBody, null, 2)});`,
  };

  const del: DocEndpoint = {
    id: "delete",
    label: "Delete record",
    method: "DELETE",
    path: `/api/collections/${name}/records/:id`,
    rule: deleteRule,
    headers: headersFor(deleteRule),
    responseStatus: 204,
    curl: curlFor("DELETE", `${base}/records/RECORD_ID`, headersFor(deleteRule)),
    js: `await cb.collection("${name}").delete("RECORD_ID");`,
  };

  // Views are read-only: `create_record`/`update_record`/`delete_record`
  // in `crates/server/src/routes/records.rs` reject any write against a
  // view collection with a 400 for every caller, superusers included, so
  // there is no write panel to show.
  if (collection.type === "view") return [list, view];

  const crud = [list, view, create, update, del];
  if (collection.type !== "auth") return crud;

  const authRule: RuleInfo = {
    value: undefined,
    tone: "fixed",
    summary: "Always reachable — credentials (or a valid refresh token) are the gate, not a rule.",
  };

  const authBody = {
    identity: identityField === "email" ? "someone@example.com" : "someuser",
    password: "a-password",
  };

  const passwordAuthEnabled = collection.passwordAuth?.enabled ?? false;
  // `client.auth` is bound to "users" by default; any other auth
  // collection needs the explicit `.as(name)` seam.
  const authVar = name === "users" ? "cb.auth" : `cb.auth.as("${name}")`;
  const authWithPassword: DocEndpoint = {
    id: "auth-with-password",
    label: "Auth with password",
    method: "POST",
    path: `/api/collections/${name}/auth-with-password`,
    rule: passwordAuthEnabled
      ? authRule
      : { ...authRule, tone: "locked", summary: "Password auth is disabled for this collection — every request is rejected with 403 PASSWORD_DISABLED." },
    headers: headersFor({ ...authRule, tone: "public" }, [CONTENT_TYPE_JSON]),
    requestBody: authBody,
    responseStatus: passwordAuthEnabled ? 200 : 403,
    responseBody: passwordAuthEnabled
      ? { token: "JWT_TOKEN", record }
      : { status: 403, message: "Password authentication is not allowed for this collection.", data: {} },
    curl: curlFor("POST", `${base}/auth-with-password`, [CONTENT_TYPE_JSON], authBody),
    js: `const auth = await ${authVar}.signIn.password({
  identity: "${authBody.identity}",
  password: "${authBody.password}",
});
${authVar}.isValid; // true`,
  };

  const authRefresh: DocEndpoint = {
    id: "auth-refresh",
    label: "Auth refresh",
    method: "POST",
    path: `/api/collections/${name}/auth-refresh`,
    rule: { value: null, tone: "locked", field: undefined, summary: "Requires a currently-valid auth token for this collection." },
    headers: headersFor({ value: null, tone: "locked", summary: "" }),
    responseStatus: 200,
    responseBody: { token: "JWT_TOKEN", record },
    curl: curlFor("POST", `${base}/auth-refresh`, headersFor({ value: null, tone: "locked", summary: "" })),
    js: `const auth = await ${authVar}.refresh();`,
  };

  const authMethods: DocEndpoint = {
    id: "auth-methods",
    label: "List auth methods",
    method: "GET",
    path: `/api/collections/${name}/auth-methods`,
    rule: { ...authRule, summary: "Always reachable — used to discover how this collection lets people sign in." },
    headers: [],
    responseStatus: 200,
    responseBody: {
      password: { enabled: passwordAuthEnabled, identityFields: collection.passwordAuth?.identityFields ?? ["email"] },
      oauth2: {
        enabled: collection.oauth2?.enabled ?? false,
        providers: (collection.oauth2?.providers ?? []).map((p) => ({ name: p.name, displayName: p.name })),
      },
      mfa: { enabled: collection.mfa?.enabled ?? false, duration: collection.mfa?.duration ?? 0 },
      otp: { enabled: collection.otp?.enabled ?? false, duration: collection.otp?.duration ?? 0 },
    },
    curl: curlFor("GET", `${base}/auth-methods`, []),
    js: `const methods = await ${authVar}.methods();`,
  };

  return [authWithPassword, authRefresh, authMethods, ...crud];
}
