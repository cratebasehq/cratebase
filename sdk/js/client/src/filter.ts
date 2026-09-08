/** Building `filter=`/`?filter=` expressions without hand-escaping
 * user input.
 *
 * Two tagged templates, because a filter expression interpolates two
 * different things: a **value** (a string/number/bool/Date literal,
 * which must be quoted and escaped) and a **field name** (an
 * identifier, which must never be quoted). `filter\`title = ${term}\``
 * needs the first; `filter\`${raw(sortField)} ~ ${term}\`` needs both. */
export class Raw {
  readonly value: string;
  constructor(value: string) {
    this.value = value;
  }
}

/** Marks `identifier` for interpolation into a `filter` template as a
 * bare field name rather than a quoted value. */
export function raw(identifier: string): Raw {
  return new Raw(identifier);
}

function escapeStringLiteral(value: string): string {
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function literal(value: unknown): string {
  if (value instanceof Raw) return value.value;
  if (value === null || value === undefined) return "null";
  if (value instanceof Date) return escapeStringLiteral(value.toISOString());
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (Array.isArray(value)) return `(${value.map(literal).join(",")})`;
  return escapeStringLiteral(String(value));
}

/** A tagged template that serializes every interpolated value into a
 * safe filter-expression literal — strings quoted and escaped, numbers
 * and booleans raw, `Date` as an ISO-8601 string, `null`/`undefined` as
 * `null`, arrays as a parenthesised list (`(a,b,c)`, the `?=`/`?~`
 * multi-match shape). Interpolate a bare field name with `raw()`
 * instead, never by writing it directly into the template string. */
export function filter(strings: TemplateStringsArray, ...values: unknown[]): string {
  let out = strings[0] ?? "";
  for (let i = 0; i < values.length; i++) {
    out += literal(values[i]);
    out += strings[i + 1] ?? "";
  }
  return out;
}
