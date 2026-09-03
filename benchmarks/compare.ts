#!/usr/bin/env bun
export {};
/**
 * Prints a side-by-side markdown table from two bench.ts result files.
 *
 *   bun run benchmarks/compare.ts results/cratebase.json results/pocketbase.json
 */

interface Run {
  category: string;
  concurrency: number;
  requestsPerSec: number;
  p50Ms: number;
  p95Ms: number;
  p99Ms: number;
  errors: number;
}
interface File {
  label: string;
  results: Run[];
}

const [a, b] = await Promise.all(
  process.argv.slice(2, 4).map(async (p) => (await Bun.file(p).json()) as File),
);
if (!a || !b) {
  console.error("usage: compare.ts <a.json> <b.json>");
  process.exit(1);
}

const key = (r: Run) => `${r.category}@${r.concurrency}`;
const bMap = new Map(b.results.map((r) => [key(r), r]));

console.log(
  `| Category | Conc | ${a.label} req/s | ${b.label} req/s | Ratio | ${a.label} p50/p99 ms | ${b.label} p50/p99 ms |`,
);
console.log("|---|---|---|---|---|---|---|");
for (const ra of a.results) {
  const rb = bMap.get(key(ra));
  if (!rb) continue;
  const ratio = ra.requestsPerSec / rb.requestsPerSec;
  const mark = ratio >= 1 ? "**" : "";
  console.log(
    `| ${ra.category} | ${ra.concurrency} | ${ra.requestsPerSec.toFixed(0)} | ${rb.requestsPerSec.toFixed(0)} | ` +
      `${mark}${ratio.toFixed(2)}x${mark} | ${ra.p50Ms.toFixed(2)} / ${ra.p99Ms.toFixed(2)} | ` +
      `${rb.p50Ms.toFixed(2)} / ${rb.p99Ms.toFixed(2)} |`,
  );
}
const errs = [...a.results, ...b.results].reduce((n, r) => n + r.errors, 0);
console.log(`\nTotal errors across both: ${errs}`);
