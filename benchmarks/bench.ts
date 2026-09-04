#!/usr/bin/env bun
/**
 * Cratebase vs PocketBase benchmark driver.
 *
 * Runs the same fixed sequence of HTTP calls against a single running
 * server (either product — this script only speaks plain HTTP/JSON, it
 * does not import either SDK) and reports p50/p95/p99 latency plus
 * requests/sec per category+concurrency combination.
 *
 * Categories (the first four mirror https://github.com/pocketbase/benchmarks):
 *   - create:      insert records (also populates the DB for search)
 *   - auth:        repeated password logins against the built-in superuser
 *   - search:      paginated list requests, perPage=30, anonymous
 *   - search-wide: same, perPage=200 (weights the per-row cost)
 *   - search-auth: same as search, with the superuser bearer token
 *                  (exercises the authenticated request path)
 *   - delete:      remove the records this run created
 *
 * Every category × concurrency cell is preceded by a discarded warm-up and
 * run `--repeats` times; the reported numbers are the run with the median
 * requests/sec, and every run is kept in the JSON output.
 *
 * Usage:
 *   bun run benchmarks/bench.ts --base-url=http://localhost:8090 \
 *     --admin-email=admin@bench.dev --admin-password=benchpass123 \
 *     [--collection=posts] [--create-n=500] [--auth-n=200] [--search-n=300] \
 *     [--concurrency=1,20,50,100] [--repeats=3] [--warmup=50] \
 *     [--out=benchmarks/results/name.json] [--label=Cratebase]
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
  repeats: number;
  warmup: number;
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
    concurrency: (map.get("concurrency") ?? "1,20,50,100").split(",").map(Number),
    repeats: Number(map.get("repeats") ?? 3),
    warmup: Number(map.get("warmup") ?? 50),
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

interface RunStats {
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

interface CategoryResult extends RunStats {
  category: string;
  concurrency: number;
  /** Every repeat, in execution order. The top-level numbers are the
   * median-throughput run. */
  runs: RunStats[];
}

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  const idx = Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1);
  return sorted[Math.max(0, idx)];
}

function stats(samples: Sample[], wallClockMs: number): RunStats {
  const latencies = samples.map((s) => s.ms).sort((a, b) => a - b);
  const errors = samples.filter((s) => !s.ok).length;
  const sum = latencies.reduce((a, b) => a + b, 0);
  return {
    n: samples.length,
    errors,
    wallClockMs: Math.round(wallClockMs),
    requestsPerSec: Number((samples.length / (wallClockMs / 1000)).toFixed(1)),
    p50Ms: Number(percentile(latencies, 50).toFixed(2)),
    p95Ms: Number(percentile(latencies, 95).toFixed(2)),
    p99Ms: Number(percentile(latencies, 99).toFixed(2)),
    minMs: Number((latencies[0] ?? 0).toFixed(2)),
    maxMs: Number((latencies[latencies.length - 1] ?? 0).toFixed(2)),
    meanMs: Number((sum / (latencies.length || 1)).toFixed(2)),
  };
}

function median(runs: RunStats[]): RunStats {
  const sorted = [...runs].sort((a, b) => a.requestsPerSec - b.requestsPerSec);
  return sorted[Math.floor(sorted.length / 2)];
}

/** Runs `n` total calls to `fn`, with at most `concurrency` in flight at
 * once. `fn(i)` must resolve to a Sample; never throw (catch inside). */
async function runPool(
  n: number,
  concurrency: number,
  fn: (i: number) => Promise<Sample>,
): Promise<{ samples: Sample[]; wallClockMs: number }> {
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
  } catch {
    return { ok: false, ms: performance.now() - start, status: 0 };
  }
}

async function detectAdminAuthPath(args: Args): Promise<string> {
  if (args.adminAuthPath) return args.adminAuthPath;
  const cratebasePath = "/api/admins/auth-with-password";
  const res = await fetch(args.baseUrl + cratebasePath, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: args.adminEmail, password: args.adminPassword }),
  });
  await res.arrayBuffer();
  if (res.ok) return cratebasePath;
  return "/api/collections/_superusers/auth-with-password";
}

async function adminLogin(args: Args, path: string): Promise<{ token: string; body: unknown }> {
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
  return { token: json.token, body };
}

/** Warm-up (discarded) followed by `repeats` measured runs. `mk(run)`
 * returns the per-request function for that run (`run === -1` is the
 * warm-up), so destructive categories can hand each run its own slice
 * of ids. */
async function measure(
  args: Args,
  category: string,
  concurrency: number,
  n: number,
  mk: (run: number) => (i: number) => Promise<Sample>,
): Promise<CategoryResult> {
  if (args.warmup > 0) {
    await runPool(args.warmup, concurrency, mk(-1));
  }
  const runs: RunStats[] = [];
  for (let r = 0; r < args.repeats; r++) {
    const { samples, wallClockMs } = await runPool(n, concurrency, mk(r));
    runs.push(stats(samples, wallClockMs));
  }
  const best = median(runs);
  console.error(
    `[${args.label}] ${category.padEnd(12)} c=${String(concurrency).padStart(3)}  ` +
      `${best.requestsPerSec.toFixed(1).padStart(8)} req/s  p50 ${best.p50Ms.toFixed(2)}ms  ` +
      `p99 ${best.p99Ms.toFixed(2)}ms  errs ${best.errors}  (median of ${runs.length})`,
  );
  return { category, concurrency, ...best, runs };
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  console.error(`[${args.label}] target ${args.baseUrl}`);

  const adminAuthPath = await detectAdminAuthPath(args);
  const { token: adminToken, body: loginBody } = await adminLogin(args, adminAuthPath);
  console.error(`[${args.label}] admin auth path: ${adminAuthPath}`);

  const recordsUrl = `${args.baseUrl}/api/collections/${args.collection}/records`;
  const results: CategoryResult[] = [];
  const authHeaders = { authorization: `Bearer ${adminToken}` };

  // Ids created per concurrency level (across warm-up and every repeat),
  // deleted later at the matching concurrency level.
  const createdIdsByConcurrency = new Map<number, string[]>();

  const createOne = (bucket: string[]) => async (i: number): Promise<Sample> => {
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
      if (res.ok && json.id) bucket.push(json.id);
      return { ok: res.ok, ms: performance.now() - start, status: res.status };
    } catch {
      return { ok: false, ms: performance.now() - start, status: 0 };
    }
  };

  // --- CREATE -----------------------------------------------------------
  for (const c of args.concurrency) {
    const bucket: string[] = [];
    createdIdsByConcurrency.set(c, bucket);
    results.push(await measure(args, "create", c, args.createN, () => createOne(bucket)));
  }
  const totalRows = [...createdIdsByConcurrency.values()].reduce((a, b) => a + b.length, 0);
  console.error(`[${args.label}] collection now holds ${totalRows} rows`);

  // --- AUTH ---------------------------------------------------------------
  const login = () =>
    timedFetch(args.baseUrl + adminAuthPath, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(loginBody),
    });
  for (const c of args.concurrency) {
    results.push(await measure(args, "auth", c, args.authN, () => login));
  }

  // --- SEARCH -------------------------------------------------------------
  const pages = Math.max(1, Math.floor(totalRows / 30));
  const search = (perPage: number, headers: Record<string, string>) => (i: number) => {
    const page = (i % Math.min(pages, 20)) + 1;
    return timedFetch(`${recordsUrl}?page=${page}&perPage=${perPage}`, { method: "GET", headers });
  };
  for (const c of args.concurrency) {
    results.push(await measure(args, "search", c, args.searchN, () => search(30, {})));
  }
  for (const c of args.concurrency) {
    results.push(await measure(args, "search-wide", c, args.searchN, () => search(200, {})));
  }
  for (const c of args.concurrency) {
    results.push(await measure(args, "search-auth", c, args.searchN, () => search(30, authHeaders)));
  }

  // --- DELETE -------------------------------------------------------------
  // Deletes are destructive, so the warm-up and each repeat consume their
  // own slice of the ids created at this concurrency level.
  for (const c of args.concurrency) {
    const ids = createdIdsByConcurrency.get(c) ?? [];
    let cursor = 0;
    const take = (count: number) => {
      const slice = ids.slice(cursor, cursor + count);
      cursor += count;
      return slice;
    };
    const del = (slice: string[]) => (i: number) =>
      i < slice.length
        ? timedFetch(`${recordsUrl}/${slice[i]}`, { method: "DELETE", headers: authHeaders })
        : Promise.resolve({ ok: false, ms: 0, status: 0 });
    const warm = take(args.warmup);
    results.push(
      await measure(args, "delete", c, args.createN, (run) => del(run < 0 ? warm : take(args.createN))),
    );
    // Remove anything left over so the collection ends empty.
    const rest = take(ids.length);
    await runPool(rest.length, Math.max(c, 20), del(rest));
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
      repeats: args.repeats,
      warmup: args.warmup,
    },
    results,
  };

  console.log(JSON.stringify(output, null, 2));
  if (args.out) {
    await Bun.write(args.out, JSON.stringify(output, null, 2));
    console.error(`[${args.label}] wrote ${args.out}`);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
