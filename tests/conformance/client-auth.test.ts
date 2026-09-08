import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { createClient, CratebaseError, type CratebaseClient } from "@cratebase/client";
import { ADMIN_EMAIL, ADMIN_PASSWORD, BASE_URL, COOKIE_MODE, uniq } from "./harness";

/**
 * `@cratebase/client`'s auth surface — sessions, sign-out, bans,
 * impersonation, cookie sessions and CSRF, and the server-driven OAuth2
 * redirect flow. None of this exists in the official `pocketbase` SDK
 * (`auth.test.ts` and the rest of this directory prove PocketBase-wire
 * compatibility; this file proves the Cratebase-only additions on top of
 * it), so it is driven entirely through `@cratebase/client` and raw
 * `fetch` where a header/cookie needs inspecting directly.
 */

async function expectCbError(fn: () => Promise<unknown>): Promise<CratebaseError> {
  try {
    await fn();
  } catch (e) {
    if (e instanceof CratebaseError) return e;
    throw new Error(`expected CratebaseError, got ${String(e)}`);
  }
  throw new Error("expected the call to throw, but it resolved");
}

const USERS = uniq("clientauthusers");
const PASS = "Password123!";

function admin(): CratebaseClient {
  return createClient(BASE_URL, { authCollection: "_superusers" });
}

/** A client whose `.auth` is bound to the throwaway `USERS` collection —
 * every plain `createClient(BASE_URL)` defaults to the built-in `"users"`
 * collection, which none of this file's test records live in. */
function freshUser(headers?: Record<string, string>): CratebaseClient {
  return createClient(BASE_URL, { authCollection: USERS, headers });
}

beforeAll(async () => {
  const su = admin();
  await su.auth.signIn.password({ identity: ADMIN_EMAIL, password: ADMIN_PASSWORD });
  await su.admin.collections.create({
    name: USERS,
    type: "auth",
    listRule: "",
    viewRule: "",
    createRule: "",
    updateRule: "id = @request.auth.id",
    deleteRule: "id = @request.auth.id",
    fields: [],
  });
});

afterAll(async () => {
  const su = admin();
  await su.auth.signIn.password({ identity: ADMIN_EMAIL, password: ADMIN_PASSWORD });
  await su.admin.collections.delete(USERS).catch(() => {});
});

/** A fresh record in the throwaway auth collection above, with its email
 * already narrowed to `string` — every other field on a generic
 * `RecordModel` is `unknown`, but `identity:` needs a real `string`. */
async function makeUser(tag: string): Promise<{ id: string; email: string }> {
  const su = admin();
  await su.auth.signIn.password({ identity: ADMIN_EMAIL, password: ADMIN_PASSWORD });
  const email = `${tag}_${USERS}@conformance.test`;
  const record = await su.collection(USERS).create({ email, password: PASS, passwordConfirm: PASS });
  return { id: record.id, email };
}

describe("client auth: sessions", () => {
  test("signIn.password creates exactly one session row, no tokenHash exposed", async () => {
    const user = await makeUser("sessions1");
    const cb = freshUser();
    await cb.auth.signIn.password({ identity: user.email, password: PASS });

    const sessions = await cb.auth.sessions.list();
    expect(sessions.length).toBe(1);
    expect(sessions[0]!.current).toBe(true);
    expect(Object.keys(sessions[0]!)).not.toContain("tokenHash");
  });

  test("a second login is a second session; revokeOthers keeps only the caller's", async () => {
    const user = await makeUser("sessions2");
    const first = freshUser();
    await first.auth.signIn.password({ identity: user.email, password: PASS });
    // `exp` is the token's only time-varying claim (the claim set is
    // frozen, no per-login nonce — see `crates/server/src/routes/auth.rs`'s
    // module doc), and it has 1-second resolution: two logins for the same
    // record inside the same wall-clock second mint byte-identical tokens.
    // A real second device never logs in that fast; this test has to,
    // deliberately, so it must not straddle that boundary either.
    await Bun.sleep(1100);
    const second = freshUser({ "User-Agent": "conformance-other-device/1.0" });
    await second.auth.signIn.password({ identity: user.email, password: PASS });

    expect((await first.auth.sessions.list()).length).toBe(2);

    const result = await first.auth.sessions.revokeOthers();
    expect(result).toEqual({ revoked: 1 });
    expect((await first.auth.sessions.list()).length).toBe(1);

    const err = await expectCbError(() => second.auth.sessions.list());
    expect(err.status).toBe(401);
  });

  test("signOut invalidates the token server-side, unlike a client-only sign-out", async () => {
    const user = await makeUser("signout");
    const cb = freshUser();
    await cb.auth.signIn.password({ identity: user.email, password: PASS });
    const oldToken = cb.auth.token;

    await cb.auth.signOut();
    expect(cb.auth.isValid).toBe(false);

    const err = await expectCbError(() =>
      cb.send(`/api/collections/${USERS}/sessions`, { headers: { Authorization: `Bearer ${oldToken}` } }),
    );
    expect(err.status).toBe(401);
  });
});

describe("client auth: bans", () => {
  test("admin.ban blocks login with 403; admin.unban restores it", async () => {
    const user = await makeUser("ban");
    const su = admin();
    await su.auth.signIn.password({ identity: ADMIN_EMAIL, password: ADMIN_PASSWORD });

    await su.auth.admin.ban(USERS, user.id);
    const banned = freshUser();
    const err = await expectCbError(() => banned.auth.signIn.password({ identity: user.email, password: PASS }));
    expect(err.status).toBe(403);
    expect(err.message).toBe("This account is banned.");

    await su.auth.admin.unban(USERS, user.id);
    const restored = freshUser();
    const res = await restored.auth.signIn.password({ identity: user.email, password: PASS });
    expect(res.record.id).toBe(user.id);
  });
});

describe("client auth: impersonation", () => {
  test("impersonate mints a non-refreshable token; refresh echoes it back; the caller's own token is untouched", async () => {
    const user = await makeUser("impersonate");
    const su = admin();
    await su.auth.signIn.password({ identity: ADMIN_EMAIL, password: ADMIN_PASSWORD });
    const callerToken = su.auth.token;

    const impersonated = await su.auth.admin.impersonate(USERS, user.id);
    expect(impersonated.record?.id).toBe(user.id);
    expect(impersonated.isValid).toBe(true);
    const mintedToken = impersonated.token;

    const refreshed = await impersonated.refresh();
    expect(refreshed.token).toBe(mintedToken);

    expect(su.auth.token).toBe(callerToken);
  });

  test("only a superuser may impersonate", async () => {
    const user = await makeUser("noimpersonate");
    const other = await makeUser("notallowed");
    const cb = freshUser();
    await cb.auth.signIn.password({ identity: other.email, password: PASS });
    const err = await expectCbError(() => cb.auth.admin.impersonate(USERS, user.id));
    expect(err.status).toBe(403);
  });
});

describe.skipIf(!COOKIE_MODE)("client auth: cookie sessions + CSRF", () => {
  test("signIn.password sets a cb_session cookie; a cookie-only client can read its own record", async () => {
    const user = await makeUser("cookie1");
    const res = await fetch(`${BASE_URL}/api/collections/${USERS}/auth-with-password`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ identity: user.email, password: PASS }),
    });
    expect(res.status).toBe(200);
    const setCookie = res.headers.get("set-cookie") ?? "";
    expect(setCookie).toContain("cb_session=");
    expect(setCookie.toLowerCase()).toContain("httponly");

    const sessionCookie = /cb_session=[^;]+/.exec(setCookie)![0]!;
    const cookieOnly = createClient(BASE_URL, { authCollection: USERS, cookie: sessionCookie });
    const sessions = await cookieOnly.auth.sessions.list();
    expect(sessions.length).toBeGreaterThan(0);
    expect(sessions[0]!.current).toBe(true);
  });

  test("a cross-site write with the session cookie and no bearer token is rejected", async () => {
    const user = await makeUser("csrf");
    const res = await fetch(`${BASE_URL}/api/collections/${USERS}/auth-with-password`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ identity: user.email, password: PASS }),
    });
    const sessionCookie = /cb_session=[^;]+/.exec(res.headers.get("set-cookie") ?? "")![0]!;

    const evil = createClient(BASE_URL, { cookie: sessionCookie, headers: { Origin: "https://evil.example" } });
    const err = await expectCbError(() => evil.collection(USERS).update(user.id, { name: "hijacked" }));
    expect(err.status).toBe(403);
    expect(err.message).toBe("Cross-site request rejected.");
  });
});

describe("client auth: oauth2 redirect flow", () => {
  test("an untrusted redirect target answers 400 JSON instead of redirecting", async () => {
    const res = await fetch(
      `${BASE_URL}/api/collections/${USERS}/oauth2/google/start?redirect=${encodeURIComponent("https://evil.example")}`,
      { redirect: "manual" },
    );
    expect(res.status).toBe(400);
    const body = (await res.json()) as { message: string };
    expect(body.message).toBe("Untrusted redirect target.");
  });

  test("a trusted redirect with oauth2 disabled on the collection bounces back with cb_error=oauth2_disabled", async () => {
    const redirect = `${BASE_URL}/done`;
    const res = await fetch(
      `${BASE_URL}/api/collections/${USERS}/oauth2/google/start?redirect=${encodeURIComponent(redirect)}`,
      { redirect: "manual" },
    );
    expect(res.status).toBe(303);
    const location = res.headers.get("location") ?? "";
    expect(location.startsWith(redirect)).toBe(true);
    expect(location).toContain("cb_error=oauth2_disabled");
  });

  test("oauth2 enabled but the requested provider isn't configured bounces back with cb_error=provider_not_enabled", async () => {
    const su = admin();
    await su.auth.signIn.password({ identity: ADMIN_EMAIL, password: ADMIN_PASSWORD });
    await su.admin.collections.update(USERS, { oauth2: { enabled: true, providers: [] } });
    try {
      const redirect = `${BASE_URL}/done`;
      const res = await fetch(
        `${BASE_URL}/api/collections/${USERS}/oauth2/google/start?redirect=${encodeURIComponent(redirect)}`,
        { redirect: "manual" },
      );
      expect(res.status).toBe(303);
      expect(res.headers.get("location") ?? "").toContain("cb_error=provider_not_enabled");
    } finally {
      await su.admin.collections.update(USERS, { oauth2: { enabled: false, providers: [] } });
    }
  });
});
