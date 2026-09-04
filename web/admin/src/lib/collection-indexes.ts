/**
 * PocketBase stores a collection's indexes as raw SQL: an `indexes: string[]`
 * of `CREATE INDEX` statements the server replays against SQLite. That is
 * the only place a UNIQUE constraint can come from in v0.23+ — there is no
 * per-field `unique` flag any more.
 *
 * These helpers read and write that string form so the editor can work
 * structurally where it can, and hand back the statement verbatim where it
 * can't (the server accepts anything SQLite does).
 */

export interface ParsedIndex {
  name: string;
  unique: boolean;
  columns: string[];
  /** The partial-index predicate, without the `WHERE`. */
  where: string;
}

const INDEX_RE =
  /^\s*CREATE\s+(UNIQUE\s+)?INDEX\s+(?:IF\s+NOT\s+EXISTS\s+)?[`"[]?([\w.-]+)[`"\]]?\s+ON\s+[`"[]?([\w.-]+)[`"\]]?\s*\(([^)]*)\)\s*(?:WHERE\s+([\s\S]+?))?\s*;?\s*$/i;

export function parseIndex(statement: string): ParsedIndex | null {
  const match = INDEX_RE.exec(statement);
  if (!match) return null;
  const columns = (match[4] ?? "")
    .split(",")
    .map((part) => part.trim().replace(/^[`"[]|[`"\]]$/g, ""))
    .filter(Boolean);
  return {
    name: match[2] ?? "",
    unique: Boolean(match[1]),
    columns,
    where: (match[5] ?? "").trim(),
  };
}

export function buildIndex(collectionName: string, index: ParsedIndex): string {
  const cols = index.columns.map((c) => `\`${c}\``).join(", ");
  const where = index.where.trim() ? ` WHERE ${index.where.trim()}` : "";
  return `CREATE ${index.unique ? "UNIQUE " : ""}INDEX \`${index.name}\` ON \`${collectionName}\` (${cols})${where}`;
}

/** `idx_posts_slug`, `idx_posts_author_created` — the shape PocketBase's own
 * generated indexes use, so hand-made and generated ones sort together. */
export function suggestIndexName(collectionName: string, columns: string[]): string {
  const suffix = columns.length > 0 ? columns.join("_") : "field";
  return `idx_${collectionName}_${suffix}`.slice(0, 64);
}

/** The errors the server would otherwise return as an opaque 400 at save. */
export function validateIndex(index: ParsedIndex, otherStatements: string[]): string | null {
  if (!index.name.trim()) return "An index needs a name";
  if (!/^[A-Za-z_][\w-]*$/.test(index.name)) return "Letters, digits, underscore; can't start with a digit";
  if (index.columns.length === 0) return "Pick at least one column";
  if (otherStatements.some((s) => parseIndex(s)?.name.toLowerCase() === index.name.toLowerCase())) {
    return "Another index already uses this name";
  }
  return null;
}
