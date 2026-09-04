import type { FieldSchema } from "@/lib/field-types";
import { isMultiValue } from "@/lib/field-types";

/**
 * A client-side mirror of the field constraints the server enforces.
 *
 * The point is not to replace server validation — it can't, the server is
 * the only authority — but to stop the drawer from being a submit-and-pray
 * form. Every rule here corresponds to one the API would reject with a 400,
 * so nothing this passes can be rejected for a reason the form could have
 * shown first, and nothing this rejects would have been accepted.
 */

/** What actually sits in the form for one field, before it's serialised. */
export type DraftValue = unknown;

export interface FileDraft {
  /** Filenames already on the record that are being kept. */
  keep: string[];
  /** Newly picked files, not yet uploaded. */
  added: File[];
}

function isBlank(value: unknown): boolean {
  if (value === null || value === undefined) return true;
  if (typeof value === "string") return value.trim().length === 0;
  if (Array.isArray(value)) return value.length === 0;
  return false;
}

function asArray(value: unknown): string[] {
  if (Array.isArray(value)) return value.map(String);
  if (value === null || value === undefined || value === "") return [];
  return [String(value)];
}

/** Bytes → "5 MB", for messages about a limit the user has to act on. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} MB`;
}

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function domainOf(email: string): string {
  return email.slice(email.lastIndexOf("@") + 1).toLowerCase();
}

/** One field's error, or `null` when it would be accepted. */
export function validateFieldValue(field: FieldSchema, value: DraftValue): string | null {
  const multiple = isMultiValue(field);

  if (field.type === "file") {
    const draft = (value ?? { keep: [], added: [] }) as FileDraft;
    const total = draft.keep.length + draft.added.length;
    if (field.required && total === 0) return "A file is required";
    const maxSelect = multiple ? Number(field.maxSelect ?? 99) : 1;
    if (total > maxSelect) return `At most ${maxSelect} ${maxSelect === 1 ? "file" : "files"}`;
    const maxSize = Number(field.maxSize ?? 0);
    const mimeTypes = (field.mimeTypes as string[] | undefined) ?? [];
    for (const file of draft.added) {
      if (maxSize > 0 && file.size > maxSize) {
        return `${file.name} is ${formatBytes(file.size)} — the limit is ${formatBytes(maxSize)}`;
      }
      if (mimeTypes.length > 0 && !mimeTypes.includes(file.type)) {
        return `${file.name} is ${file.type || "an unknown type"}; allowed: ${mimeTypes.join(", ")}`;
      }
    }
    return null;
  }

  // `autodate` is computed server-side and `bool` is never empty, so
  // neither can be "missing".
  if (field.type === "autodate" || field.type === "bool") return null;

  if (isBlank(value)) return field.required ? "This field is required" : null;

  switch (field.type) {
    case "text":
    case "editor":
    case "password": {
      const text = String(value);
      const min = Number(field.min ?? 0);
      const max = Number(field.max ?? 0);
      if (min > 0 && text.length < min) return `At least ${min} characters`;
      if (max > 0 && text.length > max) return `At most ${max} characters (currently ${text.length})`;
      const pattern = field.pattern as string | undefined;
      if (pattern) {
        try {
          if (!new RegExp(pattern).test(text)) return `Doesn't match ${pattern}`;
        } catch {
          // An invalid pattern is the schema's problem, not this value's.
        }
      }
      return null;
    }

    case "number": {
      const n = Number(value);
      if (Number.isNaN(n)) return "Must be a number";
      if (field.onlyInt && !Number.isInteger(n)) return "Must be a whole number";
      if (field.min !== undefined && field.min !== null && n < Number(field.min)) {
        return `Must be at least ${field.min}`;
      }
      if (field.max !== undefined && field.max !== null && n > Number(field.max)) {
        return `Must be at most ${field.max}`;
      }
      return null;
    }

    case "email": {
      const email = String(value);
      if (!EMAIL_RE.test(email)) return "Not a valid email address";
      const only = (field.onlyDomains as string[] | undefined) ?? [];
      const except = (field.exceptDomains as string[] | undefined) ?? [];
      const domain = domainOf(email);
      if (only.length > 0 && !only.map((d) => d.toLowerCase()).includes(domain)) {
        return `Only ${only.join(", ")} allowed`;
      }
      if (except.map((d) => d.toLowerCase()).includes(domain)) return `${domain} is not allowed`;
      return null;
    }

    case "url": {
      const raw = String(value);
      let parsed: URL;
      try {
        parsed = new URL(raw);
      } catch {
        return "Not a valid URL — include the scheme, e.g. https://";
      }
      const only = (field.onlyDomains as string[] | undefined) ?? [];
      const except = (field.exceptDomains as string[] | undefined) ?? [];
      const host = parsed.hostname.toLowerCase();
      if (only.length > 0 && !only.map((d) => d.toLowerCase()).includes(host)) {
        return `Only ${only.join(", ")} allowed`;
      }
      if (except.map((d) => d.toLowerCase()).includes(host)) return `${host} is not allowed`;
      return null;
    }

    case "date": {
      const date = new Date(String(value));
      if (Number.isNaN(date.getTime())) return "Not a valid date";
      const min = field.min as string | undefined;
      const max = field.max as string | undefined;
      if (min && date < new Date(min)) return `Must be on or after ${new Date(min).toLocaleString()}`;
      if (max && date > new Date(max)) return `Must be on or before ${new Date(max).toLocaleString()}`;
      return null;
    }

    case "select": {
      const allowed = (field.values as string[] | undefined) ?? [];
      const picked = asArray(value);
      const unknown = picked.find((v) => !allowed.includes(v));
      if (unknown) return `"${unknown}" is not one of the allowed values`;
      if (multiple) {
        const maxSelect = Number(field.maxSelect ?? 0);
        if (maxSelect > 0 && picked.length > maxSelect) return `At most ${maxSelect} selections`;
      }
      return null;
    }

    case "relation": {
      const picked = asArray(value);
      const minSelect = Number(field.minSelect ?? 0);
      const maxSelect = multiple ? Number(field.maxSelect ?? 0) : 1;
      if (minSelect > 0 && picked.length < minSelect) return `At least ${minSelect} related records`;
      if (maxSelect > 0 && picked.length > maxSelect) return `At most ${maxSelect} related records`;
      return null;
    }

    case "json": {
      // The editor holds JSON as text so a half-typed document doesn't
      // vanish; it only becomes a value once it parses.
      if (typeof value === "string") {
        try {
          JSON.parse(value);
        } catch (error) {
          return error instanceof Error ? error.message.replace(/^JSON\.parse: /, "") : "Not valid JSON";
        }
      }
      const maxSize = Number(field.maxSize ?? 0);
      if (maxSize > 0) {
        const size = new Blob([typeof value === "string" ? value : JSON.stringify(value)]).size;
        if (size > maxSize) return `JSON is ${formatBytes(size)} — the limit is ${formatBytes(maxSize)}`;
      }
      return null;
    }

    default:
      return null;
  }
}

/** Every field error in the draft, keyed by field name. */
export function validateRecordDraft(
  fields: FieldSchema[],
  values: Record<string, DraftValue>,
): Record<string, string> {
  const errors: Record<string, string> = {};
  for (const field of fields) {
    const error = validateFieldValue(field, values[field.name]);
    if (error) errors[field.name] = error;
  }
  return errors;
}

/**
 * English-ish singular of a collection name, for "New post" / "New address".
 * The old version chopped a trailing `s` and produced "New addres".
 */
export function singularize(name: string): string {
  const lower = name.toLowerCase();
  if (/(?:s|x|z|ch|sh)es$/.test(lower)) return name.slice(0, -2);
  if (/[^aeiou]ies$/.test(lower)) return `${name.slice(0, -3)}y`;
  if (/(?:ss|us|is)$/.test(lower)) return name;
  if (/s$/.test(lower)) return name.slice(0, -1);
  return name;
}
