/**
 * Conformance harness.
 *
 * Loaded once per `bun test` run (see bunfig.toml `preload`) and also imported
 * by every test file for the shared helpers. Responsibilities:
 *
 *  1. Decide which server to talk to:
 *       - `BASE_URL`               -> use an already-running server (default http://127.0.0.1:8090)
 *       - `SERVER_BIN` / `PB_BIN`  -> spawn that binary ourselves on a free port
 *                                     with a fresh temp data dir, create the superuser
 *                                     through its CLI, and kill it when the run ends.
 *  2. Expose `BASE_URL`, `ADMIN_EMAIL`, `ADMIN_PASSWORD` and a handful of helpers.
 *
 * Supported binaries (detected by file name):
 *   - PocketBase : `superuser create <email> <pass> --dir <dir>` then
 *                  `serve --http=127.0.0.1:<port> --dir <dir>`
 *   - Cratebase  : `superuser create <email> <pass>` then `serve`, configured with
 *                  the env vars CRATEBASE_DATA_DIR, DATABASE_URL and PORT.
 */
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { createServer } from "node:net";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import PocketBase, { ClientResponseError } from "pocketbase";

export const ADMIN_EMAIL = process.env.ADMIN_EMAIL || "admin@conformance.test";
export const ADMIN_PASSWORD = process.env.ADMIN_PASSWORD || "conformance123";

const SERVER_BIN = process.env.SERVER_BIN || process.env.PB_BIN || "";
const KEEP_DATA = !!process.env.KEEP_DATA;
const VERBOSE = !!process.env.CONFORMANCE_VERBOSE;

type Flavor = "pocketbase" | "cratebase";

function detectFlavor(bin: string): Flavor {
  const name = basename(bin).toLowerCase();
  if (name.includes("pocketbase")) return "pocketbase";
  return "cratebase";
}

async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = createServer();
    srv.on("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const addr = srv.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      srv.close(() => resolve(port));
    });
  });
}

async function waitForHealth(url: string, proc: ChildProcess, timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs;
  let lastErr: unknown;
  while (Date.now() < deadline) {
    if (proc.exitCode !== null) {
      throw new Error(`server exited early with code ${proc.exitCode}`);
    }
    try {
      const res = await fetch(`${url}/api/health`);
      if (res.ok) return;
      lastErr = new Error(`health returned ${res.status}`);
    } catch (e) {
      lastErr = e;
    }
    await Bun.sleep(100);
  }
  throw new Error(`server at ${url} did not become healthy: ${String(lastErr)}`);
}

interface Spawned {
  proc: ChildProcess;
  url: string;
  dir: string;
  flavor: Flavor;
}

async function spawnServer(bin: string): Promise<Spawned> {
  const flavor = detectFlavor(bin);
  const port = await freePort();
  const dir =
    process.env.SERVER_DIR || mkdtempSync(join(tmpdir(), `conformance-${flavor}-`));
  const url = `http://127.0.0.1:${port}`;

  const env: Record<string, string> = {
    ...(process.env as Record<string, string>),
    CRATEBASE_DATA_DIR: dir,
    DATABASE_URL: `sqlite://${join(dir, "data.db")}`,
    PORT: String(port),
    // Cratebase-only: httpOnly cookie sessions are boot config
    // (`crates/server/src/config.rs`), off by default because turning them
    // on rebuilds the CORS layer with `allow_credentials`. The conformance
    // run has no per-file spawn API (one server per run, see the module
    // doc), so cookie mode is enabled run-wide for this flavour only —
    // it's purely additive on top of bearer-token auth, so every existing
    // suite is unaffected. `SESSION_COOKIE_SECURE=0` because the harness
    // always talks plain HTTP to 127.0.0.1. `CORS_ALLOW_ORIGINS` is pinned
    // to this run's own origin because the CSRF gate
    // (`crates/server/src/middleware/csrf.rs`) never treats the default
    // `"*"` as a same-origin match. PocketBase ignores env vars it
    // doesn't read, so this is a no-op for the `pocketbase` flavour.
    ...(flavor === "cratebase"
      ? {
          SESSION_COOKIE: "1",
          SESSION_COOKIE_SECURE: "0",
          CORS_ALLOW_ORIGINS: url,
        }
      : {}),
  };

  // 1. create the superuser via the CLI (both binaries share the verb).
  const createArgs =
    flavor === "pocketbase"
      ? ["superuser", "create", ADMIN_EMAIL, ADMIN_PASSWORD, "--dir", dir]
      : ["superuser", "create", ADMIN_EMAIL, ADMIN_PASSWORD];
  const created = spawnSync(bin, createArgs, { env, encoding: "utf8" });
  if (created.status !== 0) {
    throw new Error(
      `superuser create failed (${created.status}): ${created.stdout}\n${created.stderr}`,
    );
  }

  // 2. serve.
  // PocketBase's automigrate (on by default) would write JS migration files to
  // `<dataDir>/../pb_migrations` and replay them on the next fresh start, so
  // disable it and confine every side directory to the temp dir.
  const serveArgs =
    flavor === "pocketbase"
      ? [
          "serve",
          `--http=127.0.0.1:${port}`,
          `--dir=${dir}`,
          "--automigrate=false",
          `--migrationsDir=${join(dir, "pb_migrations")}`,
          `--hooksDir=${join(dir, "pb_hooks")}`,
          `--publicDir=${join(dir, "pb_public")}`,
        ]
      : ["serve"];
  const proc = spawn(bin, serveArgs, {
    env,
    stdio: VERBOSE ? "inherit" : ["ignore", "ignore", "pipe"],
  });
  let stderr = "";
  proc.stderr?.on("data", (d) => {
    stderr += d.toString();
  });
  try {
    await waitForHealth(url, proc);
  } catch (e) {
    proc.kill("SIGKILL");
    throw new Error(`${String(e)}\n--- server stderr ---\n${stderr}`);
  }
  return { proc, url, dir, flavor };
}

let spawned: Spawned | null = null;

function cleanup() {
  if (!spawned) return;
  const s = spawned;
  spawned = null;
  try {
    s.proc.kill("SIGTERM");
  } catch {}
  if (!KEEP_DATA && !process.env.SERVER_DIR) {
    try {
      rmSync(s.dir, { recursive: true, force: true });
    } catch {}
  }
}

if (SERVER_BIN) {
  spawned = await spawnServer(SERVER_BIN);
  process.env.BASE_URL = spawned.url;
  process.env.SERVER_FLAVOR = spawned.flavor;
  process.on("exit", cleanup);
  process.on("SIGINT", () => {
    cleanup();
    process.exit(130);
  });
  process.on("SIGTERM", () => {
    cleanup();
    process.exit(143);
  });
}

export const BASE_URL = process.env.BASE_URL || "http://127.0.0.1:8090";

if (!spawned) {
  // No binary given: an external server must already be running and reachable.
  try {
    const res = await fetch(`${BASE_URL}/api/health`);
    if (!res.ok) throw new Error(`health returned ${res.status}`);
  } catch (e) {
    throw new Error(
      `No server at ${BASE_URL} (${String(e)}).\n` +
        `Set PB_BIN=/path/to/pocketbase or SERVER_BIN=/path/to/cratebase to let the ` +
        `harness spawn one, or point BASE_URL at a running server.`,
    );
  }
}
export const FLAVOR: Flavor =
  (process.env.SERVER_FLAVOR as Flavor) ||
  (process.env.PB_BIN ? "pocketbase" : "cratebase");

/** Whether the running server has httpOnly cookie sessions enabled — true
 * exactly when the harness spawned a Cratebase binary itself (see the
 * `SESSION_COOKIE` env block in `spawnServer` above). PocketBase has no
 * cookie-session support at all, and an externally-supplied `BASE_URL`
 * server's config is unknown, so both cases stay `false`. */
export const COOKIE_MODE = FLAVOR === "cratebase" && spawned !== null;

// ---------------------------------------------------------------------------
// SMTP sink: a minimal in-process SMTP server so that emails sent by the
// server under test (OTP, verification, password reset, email change, auth
// alerts) can be asserted on. Enabled whenever the harness spawned the server
// or MAIL_SINK=1 is set (the server must be able to reach 127.0.0.1); disable
// with MAIL_SINK=0. The harness points `settings.smtp` at it.
// ---------------------------------------------------------------------------

export interface CapturedMail {
  from: string;
  to: string[];
  subject: string;
  /** decoded text/plain part (or the whole body if not multipart) */
  text: string;
  /** decoded text/html part */
  html: string;
  raw: string;
  receivedAt: number;
}

export const mailbox: CapturedMail[] = [];

function decodeQP(s: string): string {
  return s
    .replace(/=\r?\n/g, "")
    .replace(/=([0-9A-Fa-f]{2})/g, (_, h) => String.fromCharCode(parseInt(h, 16)));
}

function decodeHeaderValue(v: string): string {
  return v.replace(/=\?([^?]+)\?([BbQq])\?([^?]*)\?=/g, (_, _cs, enc, data) =>
    enc.toUpperCase() === "B"
      ? Buffer.from(data, "base64").toString("utf8")
      : decodeQP(data.replace(/_/g, " ")),
  );
}

function decodeBody(body: string, headers: Record<string, string>): string {
  const cte = (headers["content-transfer-encoding"] || "").toLowerCase();
  if (cte === "base64") return Buffer.from(body.replace(/\s+/g, ""), "base64").toString("utf8");
  if (cte === "quoted-printable") return Buffer.from(decodeQP(body), "latin1").toString("utf8");
  return body;
}

function parseHeaders(block: string): Record<string, string> {
  const out: Record<string, string> = {};
  const unfolded = block.replace(/\r?\n[ \t]+/g, " ");
  for (const line of unfolded.split(/\r?\n/)) {
    const i = line.indexOf(":");
    if (i > 0) out[line.slice(0, i).trim().toLowerCase()] = line.slice(i + 1).trim();
  }
  return out;
}

function parseMime(raw: string): { headers: Record<string, string>; text: string; html: string } {
  const sep = raw.search(/\r?\n\r?\n/);
  const headers = parseHeaders(sep >= 0 ? raw.slice(0, sep) : raw);
  const body = sep >= 0 ? raw.slice(sep).replace(/^\r?\n\r?\n/, "") : "";
  const ct = headers["content-type"] || "text/plain";
  let text = "";
  let html = "";
  const boundary = /boundary="?([^";]+)"?/i.exec(ct)?.[1];
  if (boundary) {
    const parts = body.split(new RegExp(`--${boundary.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(?:--)?\\r?\\n?`));
    for (const part of parts) {
      if (!part.trim()) continue;
      const sub = parseMime(part);
      if (sub.text) text += sub.text;
      if (sub.html) html += sub.html;
    }
  } else if (/text\/html/i.test(ct)) {
    html = decodeBody(body, headers);
  } else {
    text = decodeBody(body, headers);
  }
  return { headers, text, html };
}

function startSmtpSink(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer((sock) => {
      let buf = "";
      let inData = false;
      let data = "";
      let from = "";
      let to: string[] = [];
      sock.write("220 conformance-sink ESMTP\r\n");
      sock.on("data", (chunk) => {
        buf += chunk.toString("utf8");
        let nl: number;
        while ((nl = buf.indexOf("\r\n")) >= 0) {
          const line = buf.slice(0, nl);
          buf = buf.slice(nl + 2);
          if (inData) {
            if (line === ".") {
              inData = false;
              const raw = data.replace(/^\.\./gm, ".");
              const m = parseMime(raw);
              mailbox.push({
                from,
                to,
                subject: decodeHeaderValue(m.headers["subject"] || ""),
                text: m.text,
                html: m.html,
                raw,
                receivedAt: Date.now(),
              });
              data = "";
              sock.write("250 OK queued\r\n");
            } else {
              data += line + "\r\n";
            }
            continue;
          }
          const cmd = line.slice(0, 4).toUpperCase();
          if (cmd === "EHLO") sock.write("250-conformance-sink\r\n250 8BITMIME\r\n");
          else if (cmd === "HELO") sock.write("250 conformance-sink\r\n");
          else if (cmd === "MAIL") {
            from = /<([^>]*)>/.exec(line)?.[1] ?? line.slice(10).trim();
            to = [];
            sock.write("250 OK\r\n");
          } else if (cmd === "RCPT") {
            to.push(/<([^>]*)>/.exec(line)?.[1] ?? line.slice(8).trim());
            sock.write("250 OK\r\n");
          } else if (cmd === "DATA") {
            inData = true;
            sock.write("354 End data with <CR><LF>.<CR><LF>\r\n");
          } else if (cmd === "QUIT") {
            sock.end("221 Bye\r\n");
          } else sock.write("250 OK\r\n");
        }
      });
      sock.on("error", () => {});
    });
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address();
      resolve(typeof addr === "object" && addr ? addr.port : 0);
    });
    server.unref();
  });
}

/** Wait for a captured email matching `pred`, then remove it from the mailbox. */
export async function waitForMail(
  pred: (m: CapturedMail) => boolean,
  { timeout = 10000, since = 0 }: { timeout?: number; since?: number } = {},
): Promise<CapturedMail> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const i = mailbox.findIndex((m) => m.receivedAt >= since && pred(m));
    if (i >= 0) return mailbox.splice(i, 1)[0]!;
    await Bun.sleep(50);
  }
  throw new Error(`waitForMail: no matching email within ${timeout}ms (mailbox has ${mailbox.length})`);
}

/** Extract an action token from a PocketBase email (`/_/#/auth/confirm-xxx/<token>`). */
export function tokenFromMail(m: CapturedMail, action: string): string {
  const re = new RegExp(`/_/#/auth/${action}/([A-Za-z0-9_.-]+)`);
  const hit = re.exec(m.html) ?? re.exec(m.text);
  if (!hit) throw new Error(`no ${action} token in mail:\n${m.raw}`);
  return hit[1]!;
}

// ---------------------------------------------------------------------------
// EventSource polyfill.
//
// Bun does not expose a global `EventSource` in the test runtime, and the SDK's
// realtime service requires one. This is a minimal fetch/streams based
// implementation covering exactly what pocketbase-js uses: named events,
// `data`, `lastEventId`, `addEventListener`, `onerror` and `close()`. Using it
// avoids adding the `eventsource` npm package as a dependency.
// ---------------------------------------------------------------------------

class SimpleEventSource {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;

  readonly url: string;
  readyState = 0;
  onerror: ((ev: unknown) => void) | null = null;
  onopen: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: unknown) => void) | null = null;

  #listeners = new Map<string, Set<(ev: unknown) => void>>();
  #controller = new AbortController();

  constructor(url: string) {
    this.url = url;
    void this.#run();
  }

  addEventListener(type: string, fn: (ev: unknown) => void) {
    let set = this.#listeners.get(type);
    if (!set) this.#listeners.set(type, (set = new Set()));
    set.add(fn);
  }

  removeEventListener(type: string, fn: (ev: unknown) => void) {
    this.#listeners.get(type)?.delete(fn);
  }

  close() {
    if (this.readyState === 2) return;
    this.readyState = 2;
    this.#controller.abort();
  }

  #emit(type: string, ev: { type: string; data: string; lastEventId: string }) {
    for (const fn of this.#listeners.get(type) ?? []) {
      try {
        fn(ev);
      } catch {}
    }
    if (type === "message") this.onmessage?.(ev);
  }

  async #run() {
    try {
      const res = await fetch(this.url, {
        headers: { accept: "text/event-stream" },
        signal: this.#controller.signal,
      });
      if (!res.ok || !res.body) throw new Error(`SSE ${res.status}`);
      this.readyState = 1;
      this.onopen?.({ type: "open" });

      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buf = "";
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        let sep: number;
        while ((sep = buf.search(/\r?\n\r?\n/)) >= 0) {
          const chunk = buf.slice(0, sep);
          buf = buf.slice(sep).replace(/^\r?\n\r?\n/, "");
          let type = "message";
          let id = "";
          const dataLines: string[] = [];
          for (const line of chunk.split(/\r?\n/)) {
            if (line.startsWith(":")) continue;
            const i = line.indexOf(":");
            const field = i < 0 ? line : line.slice(0, i);
            const value = i < 0 ? "" : line.slice(i + 1).replace(/^ /, "");
            if (field === "event") type = value;
            else if (field === "data") dataLines.push(value);
            else if (field === "id") id = value;
          }
          if (!dataLines.length && type === "message") continue;
          this.#emit(type, { type, data: dataLines.join("\n"), lastEventId: id });
        }
      }
      if (this.readyState !== 2) {
        this.readyState = 2;
        this.onerror?.({ type: "error" });
      }
    } catch (e) {
      if (this.readyState === 2) return; // closed on purpose
      this.readyState = 2;
      this.onerror?.({ type: "error", error: e });
    }
  }
}

if (typeof (globalThis as { EventSource?: unknown }).EventSource === "undefined") {
  (globalThis as { EventSource?: unknown }).EventSource = SimpleEventSource;
}

export const MAIL_SINK_ENABLED =
  process.env.MAIL_SINK !== "0" && (!!spawned || process.env.MAIL_SINK === "1");

if (MAIL_SINK_ENABLED) {
  const port = await startSmtpSink();
  process.env.MAIL_SINK_PORT = String(port);
  const pb = new PocketBase(BASE_URL);
  await pb.collection("_superusers").authWithPassword(ADMIN_EMAIL, ADMIN_PASSWORD);
  await pb.settings.update({
    meta: { appName: "Conformance", appURL: BASE_URL, senderName: "Conformance", senderAddress: "noreply@conformance.test" },
    smtp: { enabled: true, host: "127.0.0.1", port, username: "", password: "", authMethod: "", tls: false, localName: "" },
  });
}

// ---------------------------------------------------------------------------
// Helpers shared by the test files
// ---------------------------------------------------------------------------

/** A fresh, unauthenticated SDK client. */
export function client(): PocketBase {
  const pb = new PocketBase(BASE_URL);
  pb.autoCancellation(false);
  return pb;
}

/** A fresh SDK client authenticated as the superuser. */
export async function adminClient(): Promise<PocketBase> {
  const pb = client();
  await pb.collection("_superusers").authWithPassword(ADMIN_EMAIL, ADMIN_PASSWORD);
  return pb;
}

let counter = 0;
/** Unique collection / field / email suffix so test files never collide. */
export function uniq(prefix = "c"): string {
  counter += 1;
  const rand = Math.random().toString(36).slice(2, 8);
  return `${prefix}_${Date.now().toString(36)}${counter}${rand}`;
}

/** Run `fn` and return the ClientResponseError it throws (fails if it doesn't throw). */
export async function expectError(fn: () => Promise<unknown>): Promise<ClientResponseError> {
  try {
    await fn();
  } catch (e) {
    if (e instanceof ClientResponseError) return e;
    throw new Error(`expected ClientResponseError, got ${String(e)}`);
  }
  throw new Error("expected the call to throw, but it resolved");
}

/** Delete a collection, ignoring 404s (used in afterAll). */
export async function dropCollection(pb: PocketBase, idOrName: string) {
  try {
    await pb.collections.delete(idOrName);
  } catch (e) {
    if (!(e instanceof ClientResponseError) || e.status !== 404) throw e;
  }
}

/** Poll `cond` until truthy or timeout. */
export async function waitFor<T>(
  cond: () => T | Promise<T>,
  { timeout = 5000, interval = 50 } = {},
): Promise<T> {
  const deadline = Date.now() + timeout;
  let last: T;
  while (Date.now() < deadline) {
    last = await cond();
    if (last) return last;
    await Bun.sleep(interval);
  }
  throw new Error("waitFor: condition not met before timeout");
}

export { PocketBase, ClientResponseError };
