/** Cron schedule helpers shared by the job pages. */

/* ------------------------------------------------------------------ */
/* Cron next-run                                                       */
/* ------------------------------------------------------------------ */

function parseField(field: string, min: number, max: number): number[] | null {
  const out = new Set<number>();
  for (const part of field.split(",")) {
    const m = part.match(/^(\*|\d+(?:-\d+)?)(?:\/(\d+))?$/);
    if (!m) return null;
    const step = m[2] ? Number(m[2]) : 1;
    if (step < 1) return null;
    let lo: number;
    let hi: number;
    if (m[1] === "*") {
      lo = min;
      hi = max;
    } else if (m[1].includes("-")) {
      [lo, hi] = m[1].split("-").map(Number);
    } else {
      lo = Number(m[1]);
      hi = m[2] ? max : lo;
    }
    if (lo < min || hi > max || lo > hi) return null;
    for (let v = lo; v <= hi; v += step) out.add(v);
  }
  return [...out].sort((a, b) => a - b);
}

/**
 * Next UTC fire time for a five-field cron expression, or `null` when the
 * expression uses syntax this reader doesn't understand (names, `?`, `L`…).
 * The scheduler ticks on UTC, so this does too.
 */
export function nextCronRun(expression: string, from: Date = new Date()): Date | null {
  const parts = expression.trim().split(/\s+/);
  if (parts.length !== 5) return null;
  const minutes = parseField(parts[0], 0, 59);
  const hours = parseField(parts[1], 0, 23);
  const doms = parseField(parts[2], 1, 31);
  const months = parseField(parts[3], 1, 12);
  const dowsRaw = parseField(parts[4], 0, 7);
  if (!minutes || !hours || !doms || !months || !dowsRaw) return null;
  const dows = new Set(dowsRaw.map((d) => d % 7));
  const domSet = new Set(doms);
  const monthSet = new Set(months);
  const domRestricted = !parts[2].startsWith("*");
  const dowRestricted = !parts[4].startsWith("*");

  const start = Math.floor(from.getTime() / 60000) * 60000 + 60000;
  const base = new Date(start);
  const dayStart = Date.UTC(base.getUTCFullYear(), base.getUTCMonth(), base.getUTCDate());
  for (let offset = 0; offset < 366 * 5; offset++) {
    const day = new Date(dayStart + offset * 86400000);
    if (!monthSet.has(day.getUTCMonth() + 1)) continue;
    const domOk = domSet.has(day.getUTCDate());
    const dowOk = dows.has(day.getUTCDay());
    const dateOk = domRestricted && dowRestricted ? domOk || dowOk : domOk && dowOk;
    if (!dateOk) continue;
    for (const h of hours) {
      for (const m of minutes) {
        const t = day.getTime() + h * 3600000 + m * 60000;
        if (t >= start) return new Date(t);
      }
    }
  }
  return null;
}
