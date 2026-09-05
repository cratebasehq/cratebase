/** One hourly bucket as `GET /api/logs/stats` returns it. */
export interface LogBucket {
  date: string;
  total: number;
}

/**
 * `GET /api/logs/stats` only returns hours that actually have a row — an
 * hour with zero requests is simply absent, not a zero. That is fine for a
 * table, but a bar chart built directly from it degenerates: with real
 * traffic often landing in only one or two hours, "24 bars" becomes one or
 * two bars each stretched to fill nearly the whole plot, reading as a solid
 * block instead of a per-hour histogram.
 *
 * This resamples the response onto a fixed grid of `hours` consecutive
 * hourly slots ending at the current hour, filling every absent hour with
 * `total: 0`, so the chart always has the same number of categories no
 * matter how sparse the underlying data is.
 */
export function fillHourlyBuckets(stats: LogBucket[], hours = 24): LogBucket[] {
  // The server stamps buckets PocketBase-style — `2026-09-04 15:00:00.000Z`,
  // a space where an ISO string has a `T` — so both sides are reduced to
  // `YYYY-MM-DDHH` before they are compared.
  const hourKey = (value: string) => value.replace(" ", "T").slice(0, 13);
  const byHour = new Map(stats.map((b) => [hourKey(b.date), b.total]));
  const out: LogBucket[] = [];
  const now = new Date();
  now.setUTCMinutes(0, 0, 0);
  for (let i = hours - 1; i >= 0; i--) {
    const hour = new Date(now.getTime() - i * 3600_000);
    out.push({ date: hour.toISOString(), total: byHour.get(hourKey(hour.toISOString())) ?? 0 });
  }
  return out;
}
