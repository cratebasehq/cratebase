#!/usr/bin/env bun
/**
 * Cratebase vs PocketBase benchmark driver.
 *
 * Runs the same fixed sequence of HTTP calls against a single running
 * server (either product — this script only speaks plain HTTP/JSON, it
 * does not import either SDK) and reports p50/p95/p99 latency plus
 * requests/sec per category+concurrency combination.
 *
 * Categories mirror https://github.com/pocketbase/benchmarks:
 *   - create: insert records (also populates the DB for search)
 *   - auth:   repeated password logins against the built-in superuser
 *   - search: paginated list requests against the populated collection
 *   - delete: remove the records this run created
 *
 * Usage:
 *   bun run benchmarks/bench.ts --base-url=http://localhost:8090 \
 *     --admin-email=admin@bench.dev --admin-password=benchpass123 \
 *     [--collection=posts] [--create-n=500] [--auth-n=200] [--search-n=300] \
 *     [--concurrency=1,20] [--out=benchmarks/results/name.json] [--label=Cratebase]
 */

interface Args {
  baseUrl: string;
  adminEmail: string;
  adminPassword: string;
  collection: string;
  createN: number;
  authN: number;
  searchN: number;
  concurrency: number[];
  out?: string;
  label: string;
  adminAuthPath: string;
}

function parseArgs(argv: string[]): Args {
  const map = new Map<string, string>();
  for (const raw of argv) {
    const m = raw.match(/^--([^=]+)=(.*)$/);
    if (m) map.set(m[1], m[2]);
  }
  const req = (k: string): string => {
    const v = map.get(k);
    if (!v) throw new Error(`missing required --${k}`);
    return v;
  };
  return {
    baseUrl: req("base-url").replace(/\/+$/, ""),
    adminEmail: req("admin-email"),
    adminPassword: req("admin-password"),
    collection: map.get("collection") ?? "posts",
    createN: Number(map.get("create-n") ?? 500),
    authN: Number(map.get("auth-n") ?? 200),
    searchN: Number(map.get("search-n") ?? 300),
    concurrency: (map.get("concurrency") ?? "1,20").split(",").map(Number),
    out: map.get("out"),
    label: map.get("label") ?? "server",
    // Cratebase: /api/admins/auth-with-password. PocketBase: the
    // _superusers auth collection's auth-with-password. Auto-detected
    // below by probing, but can be forced with --admin-auth-path.
    adminAuthPath: map.get("admin-auth-path") ?? "",
  };
}

interface Sample {
  ok: boolean;
  ms: number;
  status: number;
}

interface CategoryResult {
  category: string;
  concurrency: number;
  n: number;
  errors: number;
  wallClockMs: number;
  requestsPerSec: number;
  p50Ms: number;
  p95Ms: number;
  p99Ms: number;
  minMs: number;
  maxMs: number;
  meanMs: number;
}

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  const idx = Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1);
  return sorted[Math.max(0, idx)];
}

function summarize(category: string, concurrency: number, samples: Sample[], wallClockMs: number): CategoryResult {
  const latencies = samples.map((s) => s.ms).sort((a, b) => a - b);
  const errors = samples.filter((s) => !s.ok).length;
  const sum = latencies.reduce((a, b) => a + b, 0);
  return {
    category,
    concurrency,
    n: samples.length,
    errors,
    wallClockMs: Math.round(wallClockMs),
    requestsPerSec: Number((samples.length / (wallClockMs / 1000)).toFixed(1)),
    p50Ms: Number(percentile(latencies, 50).toFixed(1)),
    p95Ms: Number(percentile(latencies, 95).toFixed(1)),
    p99Ms: Number(percentile(latencies, 99).toFixed(1)),
    minMs: Number((latencies[0] ?? 0).toFixed(1)),
    maxMs: Number((latencies[latencies.length - 1] ?? 0).toFixed(1)),
    meanMs: Number((sum / (latencies.length || 1)).toFixed(1)),
  };
}

/** Runs `n` total calls to `fn`, with at most `concurrency` in flight at
 * once. `fn(i)` must resolve to a Sample; never throw (catch inside). */
async function runPool(n: number, concurrency: number, fn: (i: number) => Promise<Sample>): Promise<{ samples: Sample[]; wallClockMs: number }> {
  const samples: Sample[] = new Array(n);
  let next = 0;
  const start = performance.now();
  async function worker() {
    while (true) {
      const i = next++;
      if (i >= n) return;
      samples[i] = await fn(i);
    }
  }
  const workers = Array.from({ length: Math.min(concurrency, n) }, () => worker());
  await Promise.all(workers);
  const wallClockMs = performance.now() - start;
  return { samples, wallClockMs };
}

async function timedFetch(url: string, init: RequestInit): Promise<Sample> {
  const start = performance.now();
  try {
    const res = await fetch(url, init);
    // Drain the body so the connection can be reused / the server has
    // actually finished writing the response before we stop the clock.
    await res.arrayBuffer();
    return { ok: res.ok, ms: performance.now() - start, status: res.status };
  } catch (err) {
    return { ok: false, ms: performance.now() - start, status: 0 };
  }
}

async function detectAdminAuthPath(args: Args): Promise<string> {
  if (args.adminAuthPath) return args.adminAuthPath;
  // Try Cratebase's admin path first; fall back to PocketBase's
  // _superusers auth-collection path.
  const cratebasePath = "/api/admins/auth-with-password";
  const res = await fetch(args.baseUrl + cratebasePath, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: args.adminEmail, password: args.adminPassword }),
  });
  if (res.ok) return cratebasePath;
  return "/api/collections/_superusers/auth-with-password";
}

async function adminLogin(args: Args, path: string): Promise<{ token: string; body: unknown; identityField: string }> {
  const isPb = path.includes("_superusers");
  const body = isPb
    ? { identity: args.adminEmail, password: args.adminPassword }
    : { email: args.adminEmail, password: args.adminPassword };
  const res = await fetch(args.baseUrl + path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`admin login failed: ${res.status} ${await res.text()}`);
  const json = (await res.json()) as { token: string };
  return { token: json.token, body, identityField: isPb ? "identity" : "email" };
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  console.error(`[${args.label}] target ${args.baseUrl}`);

  const adminAuthPath = await detectAdminAuthPath(args);
  const { token: adminToken, body: loginBody } = await adminLogin(args, adminAuthPath);
  console.error(`[${args.label}] admin auth path: ${adminAuthPath}`);

  const recordsUrl = `${args.baseUrl}/api/collections/${args.collection}/records`;
  const results: CategoryResult[] = [];

  // Batches of record ids created per concurrency level, deleted later at
  // the matching concurrency level.
  const createdIdsByConcurrency = new Map<number, string[]>();

  // --- CREATE -----------------------------------------------------------
  for (const c of args.concurrency) {
    const ids: string[] = new Array(args.createN);
    const { samples, wallClockMs } = await runPool(args.createN, c, async (i) => {
      const start = performance.now();
      try {
        const res = await fetch(recordsUrl, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            title: `bench post ${i}`,
            content: `benchmark content body for record ${i} — ${"x".repeat(64)}`,
            published: i % 2 === 0,
          }),
        });
        const json = (await res.json()) as { id?: string };
        if (res.ok && json.id) ids[i] = json.id;
        return { ok: res.ok, ms: performance.now() - start, status: res.status };
      } catch {
        return { ok: false, ms: performance.now() - start, status: 0 };
      }
    });
    createdIdsByConcurrency.set(c, ids.filter(Boolean));
    results.push(summarize("create", c, samples, wallClockMs));
    console.error(`[${args.label}] create c=${c} done (${ids.filter(Boolean).length}/${args.createN} ok)`);
  }

  // --- AUTH ---------------------------------------------------------------
  for (const c of args.concurrency) {
    const { samples, wallClockMs } = await runPool(args.authN, c, () => timedFetch(args.baseUrl + adminAuthPath, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(loginBody),
    }));
    results.push(summarize("auth", c, samples, wallClockMs));
    console.error(`[${args.label}] auth c=${c} done`);
  }

  // --- SEARCH ---------------------------------------------------------------
  for (const c of args.concurrency) {
    const { samples, wallClockMs } = await runPool(args.searchN, c, (i) => {
      const page = (i % 20) + 1;
      return timedFetch(`${recordsUrl}?page=${page}&perPage=30`, { method: "GET" });
    });
    results.push(summarize("search", c, samples, wallClockMs));
    console.error(`[${args.label}] search c=${c} done`);
  }

  // --- DELETE ---------------------------------------------------------------
  for (const c of args.concurrency) {
    const ids = createdIdsByConcurrency.get(c) ?? [];
    const { samples, wallClockMs } = await runPool(ids.length, c, (i) => timedFetch(`${recordsUrl}/${ids[i]}`, {
      method: "DELETE",
      headers: { authorization: `Bearer ${adminToken}` },
    }));
    results.push(summarize("delete", c, samples, wallClockMs));
    console.error(`[${args.label}] delete c=${c} done`);
  }

  const output = {
    label: args.label,
    baseUrl: args.baseUrl,
    ranAt: new Date().toISOString(),
    params: {
      collection: args.collection,
      createN: args.createN,
      authN: args.authN,
      searchN: args.searchN,
      concurrency: args.concurrency,
    },
    results,
  };

  console.log(JSON.stringify(output, null, 2));
  if (args.out) {
    await Bun.write(args.out, JSON.stringify(output, null, 2));
    console.error(`[${args.label}] wrote ${args.out}`);
  }

  // Human-readable summary table on stderr.
  console.error(`\n[${args.label}] category      conc   n     errs   req/s     p50ms   p95ms   p99ms`);
  for (const r of results) {
    console.error(
      `[${args.label}] ${r.category.padEnd(12)} ${String(r.concurrency).padStart(4)}  ${String(r.n).padStart(4)}  ${String(r.errors).padStart(4)}  ${String(r.requestsPerSec).padStart(8)}  ${String(r.p50Ms).padStart(7)}  ${String(r.p95Ms).padStart(6)}  ${String(r.p99Ms).padStart(6)}`,
    );
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
