import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import type PocketBase from "pocketbase";
import type { RecordModel } from "pocketbase";
import {
  ADMIN_EMAIL,
  ADMIN_PASSWORD,
  MAIL_SINK_ENABLED,
  adminClient,
  client,
  dropCollection,
  expectError,
  tokenFromMail,
  uniq,
  waitForMail,
} from "./harness";

/**
 * Auth flows on a dedicated auth collection.
 *
 * Emails are captured by the harness' SMTP sink (see harness.ts), which makes
 * the OTP / MFA / verification / password-reset / email-change happy paths
 * testable against PocketBase itself. Those tests are skipped when the sink is
 * disabled (MAIL_SINK=0 or an external BASE_URL without MAIL_SINK=1).
 */
let pb: PocketBase;
const USERS = uniq("authusers");
let usersId = "";
const PASS = "Password123!";
let u1: RecordModel, u2: RecordModel, u3: RecordModel;

const email = (tag: string) => `${tag}_${USERS}@conformance.test`;
const mailTest = test.skipIf(!MAIL_SINK_ENABLED);

function decodeJwt(token: string): Record<string, unknown> {
  const payload = token.split(".")[1]!;
  return JSON.parse(Buffer.from(payload, "base64url").toString("utf8"));
}

function forgedJwt(claims: Record<string, unknown>): string {
  const hdr = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url");
  const body = Buffer.from(JSON.stringify(claims)).toString("base64url");
  return `${hdr}.${body}.c2lnbmF0dXJl`;
}

const otpCode = (html: string) => /<strong>(\d+)<\/strong>/.exec(html)![1]!;

beforeAll(async () => {
  pb = await adminClient();
  const col = await pb.collections.create({
    name: USERS,
    type: "auth",
    fields: [
      { name: "name", type: "text" },
      { name: "username", type: "text" },
      { name: "role", type: "select", values: ["member", "manager"], maxSelect: 1 },
      { name: "created", type: "autodate", onCreate: true, onUpdate: false },
      { name: "updated", type: "autodate", onCreate: true, onUpdate: true },
    ],
    listRule: "",
    viewRule: "",
    createRule: "",
    // manageRule is checked IN ADDITION to updateRule, so managers must pass both
    updateRule: 'id = @request.auth.id || @request.auth.role = "manager"',
    deleteRule: "id = @request.auth.id",
    manageRule: '@request.auth.role = "manager"',
  });
  usersId = col.id;
  await pb.collections.update(col.id, {
    indexes: [...col.indexes, `CREATE UNIQUE INDEX \`idx_${USERS}_username\` ON \`${USERS}\` (\`username\`) WHERE \`username\` != ''`],
    passwordAuth: { enabled: true, identityFields: ["email", "username"] },
  });
  u1 = await pb.collection(USERS).create({ email: email("u1"), username: `u1_${USERS}`, password: PASS, passwordConfirm: PASS, name: "User One", role: "member" });
  u2 = await pb.collection(USERS).create({ email: email("u2"), username: `u2_${USERS}`, password: PASS, passwordConfirm: PASS, name: "User Two", emailVisibility: true, role: "manager" });
  u3 = await pb.collection(USERS).create({ email: email("u3"), username: `u3_${USERS}`, password: PASS, passwordConfirm: PASS, name: "User Three", role: "member" });
});

afterAll(async () => {
  await dropCollection(pb, USERS);
});

describe("auth: password", () => {
  test("authWithPassword returns token + record, hides password/tokenKey", async () => {
    const c = client();
    const res = await c.collection(USERS).authWithPassword(u1.email, PASS);
    expect(Object.keys(res).sort()).toEqual(["record", "token"]);
    expect(typeof res.token).toBe("string");
    expect(res.record.id).toBe(u1.id);
    expect(res.record.collectionName).toBe(USERS);
    expect(res.record.collectionId).toBe(usersId);
    expect(res.record.email).toBe(u1.email);
    expect(res.record.verified).toBe(false);
    expect(res.record.emailVisibility).toBe(false);
    expect(res.record).not.toHaveProperty("password");
    expect(res.record).not.toHaveProperty("tokenKey");
    expect(res.record).not.toHaveProperty("passwordConfirm");
    expect(c.authStore.isValid).toBe(true);
    expect(c.authStore.record?.id).toBe(u1.id);
    expect(c.authStore.isSuperuser).toBe(false);

    const claims = decodeJwt(res.token);
    expect(Object.keys(claims).sort()).toEqual(["collectionId", "exp", "id", "refreshable", "type"]);
    expect(claims.id).toBe(u1.id);
    expect(claims.type).toBe("auth");
    expect(claims.collectionId).toBe(usersId);
    expect(claims.refreshable).toBe(true);
    // default authToken.duration for new auth collections is 432000s (5 days)
    const ttl = (claims.exp as number) - Math.floor(Date.now() / 1000);
    expect(ttl).toBeGreaterThan(432000 - 60);
    expect(ttl).toBeLessThanOrEqual(432000);
  });

  test("wrong password / unknown identity -> 400 'Failed to authenticate.'", async () => {
    const c = client();
    let err = await expectError(() => c.collection(USERS).authWithPassword(u1.email, "wrong-password"));
    expect(err.status).toBe(400);
    expect(err.response).toEqual({ status: 400, message: "Failed to authenticate.", data: {} });
    err = await expectError(() => c.collection(USERS).authWithPassword("nobody@conformance.test", PASS));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Failed to authenticate.");
    expect(c.authStore.isValid).toBe(false);
  });

  test("empty identity / password -> field validation errors", async () => {
    const err = await expectError(() => client().collection(USERS).authWithPassword("", ""));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("An error occurred while validating the submitted data.");
    expect(err.response.data.identity.code).toBe("validation_required");
    expect(err.response.data.password.code).toBe("validation_required");
  });

  test("identityFields: login with username", async () => {
    const c = client();
    const res = await c.collection(USERS).authWithPassword(`u1_${USERS}`, PASS);
    expect(res.record.id).toBe(u1.id);
    // identityField can be pinned explicitly; pinning the wrong one fails
    const res2 = await client().collection(USERS).authWithPassword(u1.email, PASS, { body: { identity: u1.email, password: PASS, identityField: "email" } });
    expect(res2.record.id).toBe(u1.id);
    const err = await expectError(() =>
      client().collection(USERS).authWithPassword(u1.email, PASS, { body: { identity: u1.email, password: PASS, identityField: "username" } }),
    );
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Failed to authenticate.");
  });

  test("passwordAuth disabled -> 403", async () => {
    const name = uniq("nopass");
    await pb.collections.create({ name, type: "auth", fields: [], passwordAuth: { enabled: false, identityFields: ["email"] } });
    try {
      const err = await expectError(() => client().collection(name).authWithPassword("a@b.c", "x"));
      expect(err.status).toBe(403);
      expect(err.response.message).toBe("The collection is not configured to allow password authentication.");
      const methods = await client().collection(name).listAuthMethods();
      expect(methods.password.enabled).toBe(false);
    } finally {
      await dropCollection(pb, name);
    }
  });

  test("auth endpoints on a base collection -> 404", async () => {
    const name = uniq("notauth");
    await pb.collections.create({ name, type: "base", fields: [] });
    try {
      let err = await expectError(() => client().collection(name).authWithPassword("a@b.c", "x"));
      expect(err.status).toBe(404);
      expect(err.response).toEqual({ status: 404, message: "Missing or invalid auth collection context.", data: {} });
      err = await expectError(() => client().collection(name).listAuthMethods());
      expect(err.status).toBe(404);
      expect(err.response.message).toBe("Missing or invalid auth collection context.");
      err = await expectError(() => client().collection(name).requestVerification("a@b.c"));
      expect(err.status).toBe(404);
    } finally {
      await dropCollection(pb, name);
    }
  });

  test("_superusers auth", async () => {
    const c = client();
    const res = await c.collection("_superusers").authWithPassword(ADMIN_EMAIL, ADMIN_PASSWORD);
    expect(res.record.collectionName).toBe("_superusers");
    expect(res.record.collectionId).toBe("pbc_3142635823");
    expect(res.record.email).toBe(ADMIN_EMAIL);
    expect(res.record.verified).toBe(true);
    expect(c.authStore.isSuperuser).toBe(true);
    expect(decodeJwt(res.token).collectionId).toBe("pbc_3142635823");
    // guests cannot list superusers
    const err = await expectError(() => client().collection("_superusers").getList());
    expect(err.status).toBe(403);
    expect(err.response.message).toBe("Only superusers can perform this action.");
  });
});

describe("auth: refresh / token invalidation", () => {
  test("authRefresh returns a new token for the same record", async () => {
    const c = client();
    const first = await c.collection(USERS).authWithPassword(u1.email, PASS);
    await Bun.sleep(1100); // exp has second resolution -> guarantees a different token
    const res = await c.collection(USERS).authRefresh();
    expect(res.record.id).toBe(u1.id);
    expect(res.token).not.toBe(first.token);
    expect(c.authStore.token).toBe(res.token);
  });

  test("authRefresh without a token -> 401; with a token from another collection -> 403", async () => {
    let err = await expectError(() => client().collection(USERS).authRefresh());
    expect(err.status).toBe(401);
    expect(err.response).toEqual({ status: 401, message: "The request requires valid record authorization token.", data: {} });
    err = await expectError(() => pb.collection(USERS).authRefresh());
    expect(err.status).toBe(403);
    expect(err.response.message).toBe("The request requires auth record from _superusers collection.");
  });

  test("garbage token -> 401", async () => {
    const c = client();
    c.authStore.save("not.a.jwt", null);
    const err = await expectError(() => c.collection(USERS).authRefresh());
    expect(err.status).toBe(401);
  });

  test("password change invalidates existing tokens", async () => {
    const victim = await pb.collection(USERS).create({ email: email("victim"), password: PASS, passwordConfirm: PASS });
    const c = client();
    await c.collection(USERS).authWithPassword(victim.email, PASS);
    await c.collection(USERS).authRefresh(); // still valid
    // self password change requires oldPassword
    const err = await expectError(() => c.collection(USERS).update(victim.id, { password: "NewPass456!", passwordConfirm: "NewPass456!" }));
    expect(err.status).toBe(400);
    expect(err.response.data.oldPassword.code).toBe("validation_required");
    const err0 = await expectError(() => c.collection(USERS).update(victim.id, { oldPassword: "nope", password: "NewPass456!", passwordConfirm: "NewPass456!" }));
    expect(err0.response.data.oldPassword.code).toBe("validation_invalid_old_password");
    await c.collection(USERS).update(victim.id, { oldPassword: PASS, password: "NewPass456!", passwordConfirm: "NewPass456!" });
    // the SDK keeps the old token; the server no longer accepts it
    const err2 = await expectError(() => c.collection(USERS).authRefresh());
    expect(err2.status).toBe(401);
    // and the new password works
    await client().collection(USERS).authWithPassword(victim.email, "NewPass456!");
    // superuser password change also rotates tokenKey
    const c2 = client();
    await c2.collection(USERS).authWithPassword(victim.email, "NewPass456!");
    await pb.collection(USERS).update(victim.id, { password: PASS, passwordConfirm: PASS });
    const err3 = await expectError(() => c2.collection(USERS).authRefresh());
    expect(err3.status).toBe(401);
    await pb.collection(USERS).delete(victim.id);
  });

  test("password / passwordConfirm / email validation on create", async () => {
    let err = await expectError(() => pb.collection(USERS).create({ email: email("bad"), password: "short", passwordConfirm: "short" }));
    expect(err.response.data.password.code).toBe("validation_min_text_constraint");
    err = await expectError(() => pb.collection(USERS).create({ email: email("bad"), password: PASS, passwordConfirm: "different1" }));
    expect(err.response.data.passwordConfirm).toEqual({ code: "validation_values_mismatch", message: "Values don't match." });
    err = await expectError(() => pb.collection(USERS).create({ email: u1.email, password: PASS, passwordConfirm: PASS }));
    expect(err.response.data.email.code).toBe("validation_not_unique");
    err = await expectError(() => pb.collection(USERS).create({ email: "not-an-email", password: PASS, passwordConfirm: PASS }));
    expect(err.response.data.email.code).toBe("validation_is_email");
    err = await expectError(() => pb.collection(USERS).create({ email: email("nopw") }));
    expect(err.response.data.password.code).toBe("validation_required");
  });
});

describe("auth: visibility and rules", () => {
  test("emailVisibility masks email for other users/guests; self, superuser and managers see it", async () => {
    const c1 = client();
    await c1.collection(USERS).authWithPassword(u1.email, PASS);
    const list = await c1.collection(USERS).getFullList({ filter: `id = "${u1.id}" || id = "${u2.id}" || id = "${u3.id}"`, sort: "created" });
    const me = list.find((r) => r.id === u1.id)!;
    const manager = list.find((r) => r.id === u2.id)!;
    const other = list.find((r) => r.id === u3.id)!;
    expect(me.email).toBe(u1.email); // self always sees own email
    expect(manager.email).toBe(u2.email); // u2 opted in with emailVisibility=true
    expect(other).not.toHaveProperty("email"); // hidden email is omitted, not blanked

    const guest = await client().collection(USERS).getOne(u1.id);
    expect(guest).not.toHaveProperty("email");
    expect(guest.emailVisibility).toBe(false);
    expect((await pb.collection(USERS).getOne(u1.id)).email).toBe(u1.email);
    // manageRule access also reveals the email
    const cm = client();
    await cm.collection(USERS).authWithPassword(u2.email, PASS);
    expect((await cm.collection(USERS).getOne(u3.id)).email).toBe(u3.email);
  });

  test("updateRule: members cannot update others; managers can (incl. password/verified via manageRule)", async () => {
    const c1 = client();
    await c1.collection(USERS).authWithPassword(u1.email, PASS);
    const err = await expectError(() => c1.collection(USERS).update(u3.id, { name: "hacked" }));
    expect(err.status).toBe(404);

    const manager = client();
    await manager.collection(USERS).authWithPassword(u2.email, PASS);
    const upd = await manager.collection(USERS).update(u3.id, { name: "Renamed by manager", password: PASS, passwordConfirm: PASS, verified: true });
    expect(upd.name).toBe("Renamed by manager");
    expect(upd.verified).toBe(true);
    await pb.collection(USERS).update(u3.id, { name: "User Three", verified: false });
  });

  test("self-update of verified/email without manage access -> validation_values_mismatch", async () => {
    const c1 = client();
    await c1.collection(USERS).authWithPassword(u1.email, PASS);
    let err = await expectError(() => c1.collection(USERS).update(u1.id, { verified: true }));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Failed to update record.");
    expect(err.response.data.verified).toEqual({ code: "validation_values_mismatch", message: "Values don't match." });
    err = await expectError(() => c1.collection(USERS).update(u1.id, { email: email("changed") }));
    expect(err.response.data.email).toEqual({ code: "validation_values_mismatch", message: "Values don't match." });
    // but emailVisibility is a normal field
    const r = await c1.collection(USERS).update(u1.id, { emailVisibility: true });
    expect(r.emailVisibility).toBe(true);
    await c1.collection(USERS).update(u1.id, { emailVisibility: false });
  });

  test("authRule blocks login when it evaluates to false", async () => {
    await pb.collections.update(usersId, { authRule: "verified = true" });
    try {
      const err = await expectError(() => client().collection(USERS).authWithPassword(u1.email, PASS));
      expect(err.status).toBe(403);
      expect(err.response.message).toBe("The request doesn't satisfy the collection requirements to authenticate.");
      await pb.collection(USERS).update(u1.id, { verified: true });
      const ok = await client().collection(USERS).authWithPassword(u1.email, PASS);
      expect(ok.record.verified).toBe(true);
    } finally {
      await pb.collections.update(usersId, { authRule: "" });
      await pb.collection(USERS).update(u1.id, { verified: false });
    }
  });

  test("createRule lets guests sign up; response omits password and (hidden) email", async () => {
    const guest = client();
    const r = await guest.collection(USERS).create({ email: email("signup"), password: PASS, passwordConfirm: PASS });
    expect(r.id).toMatch(/^[a-z0-9]{15}$/);
    expect(r).not.toHaveProperty("password");
    expect(r).not.toHaveProperty("tokenKey");
    expect(r).not.toHaveProperty("email"); // the creator is a guest, so the hidden email is stripped
    expect(r.verified).toBe(false);
    await pb.collection(USERS).delete(r.id);
  });
});

describe("auth: verification / password reset / email change", () => {
  test("requestVerification always succeeds (even for unknown emails)", async () => {
    expect(await client().collection(USERS).requestVerification(u3.email)).toBe(true);
    expect(await client().collection(USERS).requestVerification("unknown@conformance.test")).toBe(true);
    const err = await expectError(() => client().collection(USERS).requestVerification("not-an-email"));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("An error occurred while validating the submitted data.");
    expect(err.response.data.email.code).toBe("validation_is_email");
  });

  test("confirmVerification with bad tokens", async () => {
    let err = await expectError(() => client().collection(USERS).confirmVerification("bad.token.here"));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("An error occurred while validating the submitted data.");
    expect(err.response.data.token).toEqual({ code: "validation_invalid_token_claims", message: "Missing email token claim." });
    const forged = forgedJwt({ id: u1.id, collectionId: usersId, email: u1.email, type: "verification", exp: Math.floor(Date.now() / 1000) + 3600 });
    err = await expectError(() => client().collection(USERS).confirmVerification(forged));
    expect(err.response.data.token).toEqual({ code: "validation_invalid_token", message: "Invalid or expired token." });
  });

  mailTest("verification happy path via captured mail", async () => {
    const target = await pb.collection(USERS).create({ email: email("verifyme"), password: PASS, passwordConfirm: PASS });
    expect(await client().collection(USERS).requestVerification(target.email)).toBe(true);
    const m = await waitForMail((m) => m.to.includes(target.email) && /verify/i.test(m.subject));
    expect(m.subject).toBe("Verify your Conformance email");
    expect(m.from).toBe("noreply@conformance.test");
    const token = tokenFromMail(m, "confirm-verification");
    const claims = decodeJwt(token);
    expect(claims.type).toBe("verification");
    expect(claims.email).toBe(target.email);
    expect(claims.collectionId).toBe(usersId);
    expect(await client().collection(USERS).confirmVerification(token)).toBe(true);
    expect((await pb.collection(USERS).getOne(target.id)).verified).toBe(true);
    // already verified -> requestVerification is a silent no-op (no email)
    expect(await client().collection(USERS).requestVerification(target.email)).toBe(true);
    await pb.collection(USERS).delete(target.id);
  });

  test("requestPasswordReset always succeeds; confirm with bad token fails", async () => {
    expect(await client().collection(USERS).requestPasswordReset(u3.email)).toBe(true);
    expect(await client().collection(USERS).requestPasswordReset("unknown@conformance.test")).toBe(true);
    const err = await expectError(() => client().collection(USERS).confirmPasswordReset("bad.token", "NewPass456!", "NewPass456!"));
    expect(err.status).toBe(400);
    expect(err.response.data.token).toEqual({ code: "validation_invalid_token", message: "Invalid or expired token." });
    const err2 = await expectError(() => client().collection(USERS).confirmPasswordReset("bad.token", "NewPass456!", "mismatch"));
    expect(err2.response.data.passwordConfirm.code).toBe("validation_values_mismatch");
  });

  mailTest("password reset happy path via captured mail", async () => {
    const target = await pb.collection(USERS).create({ email: email("resetme"), password: PASS, passwordConfirm: PASS });
    expect(await client().collection(USERS).requestPasswordReset(target.email)).toBe(true);
    const m = await waitForMail((m) => m.to.includes(target.email) && /reset/i.test(m.subject));
    expect(m.subject).toBe("Reset your Conformance password");
    const token = tokenFromMail(m, "confirm-password-reset");
    expect(decodeJwt(token).type).toBe("passwordReset");
    expect(await client().collection(USERS).confirmPasswordReset(token, "NewPass456!", "NewPass456!")).toBe(true);
    await client().collection(USERS).authWithPassword(target.email, "NewPass456!");
    // the token is single-use
    const err = await expectError(() => client().collection(USERS).confirmPasswordReset(token, "NewPass789!", "NewPass789!"));
    expect(err.response.data.token.code).toBe("validation_invalid_token");
    await pb.collection(USERS).delete(target.id);
  });

  test("requestEmailChange requires auth and a different email; confirm with bad token fails", async () => {
    let err = await expectError(() => client().collection(USERS).requestEmailChange(email("new")));
    expect(err.status).toBe(401);
    const c = client();
    await c.collection(USERS).authWithPassword(u3.email, PASS);
    err = await expectError(() => c.collection(USERS).requestEmailChange(u3.email));
    expect(err.status).toBe(400);
    expect(err.response.data.newEmail).toEqual({ code: "validation_not_in_invalid", message: "Must not be in list." });
    err = await expectError(() => c.collection(USERS).confirmEmailChange("bad.token", PASS));
    expect(err.status).toBe(400);
    expect(err.response.data.token).toEqual({ code: "validation_invalid_token_payload", message: "Invalid token payload - newEmail must be set." });
    expect(err.response.data.password.code).toBe("validation_invalid_password");
    const forged = forgedJwt({ id: u3.id, collectionId: usersId, email: u3.email, newEmail: email("forged"), type: "emailChange", exp: Math.floor(Date.now() / 1000) + 3600 });
    err = await expectError(() => c.collection(USERS).confirmEmailChange(forged, PASS));
    expect(err.response.data.token.code).toBe("validation_invalid_token");
  });

  mailTest("email change happy path via captured mail", async () => {
    const target = await pb.collection(USERS).create({ email: email("changeme"), password: PASS, passwordConfirm: PASS });
    const c = client();
    await c.collection(USERS).authWithPassword(target.email, PASS);
    const newEmail = email("changed");
    expect(await c.collection(USERS).requestEmailChange(newEmail)).toBe(true);
    const m = await waitForMail((m) => m.to.includes(newEmail));
    expect(m.subject).toBe("Confirm your Conformance new email address");
    const token = tokenFromMail(m, "confirm-email-change");
    expect(decodeJwt(token).newEmail).toBe(newEmail);
    const err = await expectError(() => client().collection(USERS).confirmEmailChange(token, "wrong-password"));
    expect(err.response.data.password).toEqual({ code: "validation_invalid_password", message: "Missing or invalid auth record password." });
    expect(await client().collection(USERS).confirmEmailChange(token, PASS)).toBe(true);
    const after = await pb.collection(USERS).getOne(target.id);
    expect(after.email).toBe(newEmail);
    expect(after.verified).toBe(true);
    // confirming the email change rotates tokenKey -> old session invalid
    const err2 = await expectError(() => c.collection(USERS).authRefresh());
    expect(err2.status).toBe(401);
    await pb.collection(USERS).delete(target.id);
  });
});

describe("auth: impersonate / auth methods / origins / external auths", () => {
  test("impersonate returns a non-refreshable token; authRefresh echoes it back unchanged", async () => {
    const imp = await pb.collection(USERS).impersonate(u1.id, 3600);
    expect(imp.authStore.record?.id).toBe(u1.id);
    expect(imp.authStore.isValid).toBe(true);
    const token = imp.authStore.token;
    const claims = decodeJwt(token);
    expect(claims.id).toBe(u1.id);
    expect(claims.refreshable).toBe(false);
    expect((claims.exp as number) - Math.floor(Date.now() / 1000)).toBeLessThanOrEqual(3600);
    // acts as the user
    const me = await imp.collection(USERS).getOne(u1.id);
    expect(me.email).toBe(u1.email);
    // NOTE: auth-refresh does not fail for non-refreshable tokens; it returns the same token
    const refreshed = await imp.collection(USERS).authRefresh();
    expect(refreshed.token).toBe(token);
    // only superusers may impersonate
    const c = client();
    await c.collection(USERS).authWithPassword(u2.email, PASS);
    const err = await expectError(() => c.collection(USERS).impersonate(u1.id, 60));
    expect(err.status).toBe(403);
    expect(err.response.message).toBe("The authorized record is not allowed to perform this action.");
    const err2 = await expectError(() => pb.collection(USERS).impersonate("doesnotexist000", 60));
    expect(err2.status).toBe(404);
  });

  test("listAuthMethods shape", async () => {
    const m = await client().collection(USERS).listAuthMethods();
    expect(Object.keys(m).sort()).toEqual(["mfa", "oauth2", "otp", "password"]);
    expect(m.password).toEqual({ enabled: true, identityFields: ["email", "username"] });
    expect(m.oauth2).toEqual({ enabled: false, providers: [] });
    expect(m.mfa).toEqual({ enabled: false, duration: 0 });
    expect(m.otp).toEqual({ enabled: false, duration: 0 });
    const err = await expectError(() => client().collection("does_not_exist").listAuthMethods());
    expect(err.status).toBe(404);
    expect(err.response.message).toBe("Missing or invalid collection context.");
  });

  test("_authOrigins gets a row after login and is only visible to the owner", async () => {
    const c = client();
    await c.collection(USERS).authWithPassword(u2.email, PASS);
    const origins = await c.collection("_authOrigins").getFullList();
    expect(origins.length).toBeGreaterThanOrEqual(1);
    for (const o of origins) {
      expect(o.recordRef).toBe(u2.id);
      expect(o.collectionRef).toBe(usersId);
      expect(o.fingerprint).toMatch(/^[a-f0-9]{32}$/);
    }
    // other users see nothing
    const c1 = client();
    await c1.collection(USERS).authWithPassword(u1.email, PASS);
    const others = await c1.collection("_authOrigins").getFullList({ filter: `recordRef = "${u2.id}"` });
    expect(others).toEqual([]);
    // guests: the list rule requires auth -> empty list, not an error
    expect((await client().collection("_authOrigins").getList()).totalItems).toBe(0);
    // the owner can delete its origin
    expect(await c.collection("_authOrigins").delete(origins[0]!.id)).toBe(true);
  });

  mailTest("auth alert email is sent on login from a new origin", async () => {
    const target = await pb.collection(USERS).create({ email: email("alerted"), password: PASS, passwordConfirm: PASS });
    const c = client();
    // first login on a fresh record never alerts
    await c.collection(USERS).authWithPassword(target.email, PASS);
    const origins = await c.collection("_authOrigins").getFullList();
    expect(origins).toHaveLength(1);
    // simulate a different device (different user agent fingerprint)
    const c2 = client();
    await c2.collection(USERS).authWithPassword(target.email, PASS, { headers: { "user-agent": "conformance-other-device/1.0" } });
    const m = await waitForMail((m) => m.to.includes(target.email) && /new location/i.test(m.subject));
    expect(m.subject).toBe("Login from a new location");
    expect((await c.collection("_authOrigins").getFullList()).length).toBe(2);
    await pb.collection(USERS).delete(target.id);
  });

  test("external auths: empty list / unlink 404", async () => {
    const c = client();
    await c.collection(USERS).authWithPassword(u1.email, PASS);
    expect(await c.collection(USERS).listExternalAuths(u1.id)).toEqual([]);
    const err = await expectError(() => c.collection(USERS).unlinkExternalAuth(u1.id, "google"));
    expect(err.status).toBe(404);
  });
});

describe("auth: OTP", () => {
  beforeAll(async () => {
    await pb.collections.update(usersId, { otp: { enabled: true, duration: 180, length: 8 } });
  });
  afterAll(async () => {
    await pb.collections.update(usersId, { otp: { enabled: false, duration: 180, length: 8 } });
  });

  test("listAuthMethods reflects otp", async () => {
    const m = await client().collection(USERS).listAuthMethods();
    expect(m.otp).toEqual({ enabled: true, duration: 180 });
  });

  test("requestOTP returns an otpId (also for unknown emails); wrong code / unknown id -> 400", async () => {
    const res = await client().collection(USERS).requestOTP(u3.email);
    expect(Object.keys(res)).toEqual(["otpId"]);
    expect(res.otpId).toMatch(/^[a-z0-9]{15}$/);
    const fake = await client().collection(USERS).requestOTP("unknown@conformance.test");
    expect(fake.otpId).toMatch(/^[a-z0-9]{15}$/);

    let err = await expectError(() => client().collection(USERS).authWithOTP(res.otpId, "00000000"));
    expect(err.status).toBe(400);
    expect(err.response).toEqual({ status: 400, message: "Invalid or expired OTP.", data: {} });
    err = await expectError(() => client().collection(USERS).authWithOTP("nonexistent0001", "00000000"));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Invalid or expired OTP.");
    err = await expectError(() => client().collection(USERS).authWithOTP("", ""));
    expect(err.response.data.otpId.code).toBe("validation_required");
    expect(err.response.data.password.code).toBe("validation_required");

    // the owner can see their pending _otps row (without password/sentTo)
    const c = client();
    await c.collection(USERS).authWithPassword(u3.email, PASS);
    const otps = await c.collection("_otps").getFullList();
    expect(otps.some((o) => o.id === res.otpId)).toBe(true);
    expect(otps[0]).not.toHaveProperty("password");
    expect(otps[0]).not.toHaveProperty("sentTo");
    expect(otps[0]!.recordRef).toBe(u3.id);
  });

  test("OTP endpoints when otp is disabled -> 403", async () => {
    const name = uniq("nootp");
    await pb.collections.create({ name, type: "auth", fields: [] });
    try {
      let err = await expectError(() => client().collection(name).requestOTP("a@b.c"));
      expect(err.status).toBe(403);
      expect(err.response.message).toBe("The collection is not configured to allow OTP authentication.");
      err = await expectError(() => client().collection(name).authWithOTP("aaaaaaaaaaaaaaa", "000"));
      expect(err.status).toBe(403);
    } finally {
      await dropCollection(pb, name);
    }
  });

  mailTest("authWithOTP happy path via captured mail (marks the record verified; single use)", async () => {
    const target = await pb.collection(USERS).create({ email: email("otpme"), password: PASS, passwordConfirm: PASS });
    const { otpId } = await client().collection(USERS).requestOTP(target.email);
    const m = await waitForMail((m) => m.to.includes(target.email) && /OTP/.test(m.subject));
    expect(m.subject).toBe("OTP for Conformance");
    const code = otpCode(m.html);
    expect(code).toMatch(/^\d{8}$/);
    const c = client();
    const res = await c.collection(USERS).authWithOTP(otpId, code);
    expect(res.record.id).toBe(target.id);
    expect(res.record.verified).toBe(true);
    expect(decodeJwt(res.token).refreshable).toBe(true);
    const err = await expectError(() => client().collection(USERS).authWithOTP(otpId, code));
    expect(err.response.message).toBe("Invalid or expired OTP.");
    await pb.collection(USERS).delete(target.id);
  });
});

describe("auth: MFA", () => {
  beforeAll(async () => {
    // MFA needs a second auth method, so OTP has to be enabled too.
    await pb.collections.update(usersId, { otp: { enabled: true, duration: 180, length: 8 }, mfa: { enabled: true, duration: 600, rule: "" } });
  });
  afterAll(async () => {
    await pb.collections.update(usersId, { mfa: { enabled: false, duration: 600, rule: "" }, otp: { enabled: false, duration: 180, length: 8 } });
  });

  test("enabling mfa without a second auth method is rejected", async () => {
    const name = uniq("mfaonly");
    await pb.collections.create({ name, type: "auth", fields: [] });
    try {
      const err = await expectError(() => pb.collections.update(name, { mfa: { enabled: true, duration: 600, rule: "" } }));
      expect(err.status).toBe(400);
      expect(err.response.data.mfa.enabled.code).toBe("validation_mfa_not_enough_auths");
    } finally {
      await dropCollection(pb, name);
    }
  });

  test("first factor returns 401 {mfaId}; same method again -> 400; unknown mfaId -> 400", async () => {
    const c = client();
    const err = await expectError(() => c.collection(USERS).authWithPassword(u1.email, PASS));
    expect(err.status).toBe(401);
    expect(Object.keys(err.response)).toEqual(["mfaId"]);
    const mfaId = err.response.mfaId as string;
    expect(mfaId).toMatch(/^[a-z0-9]{15}$/);
    expect(c.authStore.isValid).toBe(false);

    const err2 = await expectError(() =>
      c.collection(USERS).authWithPassword(u1.email, PASS, { body: { identity: u1.email, password: PASS, mfaId } }),
    );
    // NOTE: PocketBase does NOT echo the mfaId here; re-using the same method is a 400.
    expect(err2.status).toBe(400);
    expect(err2.response).toEqual({ status: 400, message: "A different authentication method is required.", data: {} });
    // wrong password with a valid mfaId is still a plain auth failure
    const errPw = await expectError(() =>
      c.collection(USERS).authWithPassword(u1.email, "wrong", { body: { identity: u1.email, password: "wrong", mfaId } }),
    );
    expect(errPw.response.message).toBe("Failed to authenticate.");

    const row = await pb.collection("_mfas").getOne(mfaId);
    expect(row.recordRef).toBe(u1.id);
    expect(row.collectionRef).toBe(usersId);
    expect(row.method).toBe("password");

    const err3 = await expectError(() =>
      c.collection(USERS).authWithPassword(u1.email, PASS, { body: { identity: u1.email, password: PASS, mfaId: "doesnotexist000" } }),
    );
    expect(err3.status).toBe(400);
    expect(err3.response).toEqual({ status: 400, message: "Invalid or expired MFA session.", data: {} });

    const m = await client().collection(USERS).listAuthMethods();
    expect(m.mfa).toEqual({ enabled: true, duration: 600 });
  });

  mailTest("MFA completes with OTP as the second factor", async () => {
    const err = await expectError(() => client().collection(USERS).authWithPassword(u3.email, PASS));
    const mfaId = err.response.mfaId as string;
    const since = Date.now();
    const { otpId } = await client().collection(USERS).requestOTP(u3.email);
    const m = await waitForMail((m) => m.to.includes(u3.email) && /OTP/.test(m.subject), { since });
    const code = otpCode(m.html);
    // OTP alone (without mfaId) is itself only a first factor under MFA
    const c = client();
    const res = await c.collection(USERS).authWithOTP(otpId, code, { body: { otpId, password: code, mfaId } });
    expect(res.record.id).toBe(u3.id);
    expect(c.authStore.isValid).toBe(true);
    // the _mfas row is consumed
    const gone = await expectError(() => pb.collection("_mfas").getOne(mfaId));
    expect(gone.status).toBe(404);
  });

  mailTest("OTP alone under MFA also returns 401 {mfaId}", async () => {
    const since = Date.now();
    const { otpId } = await client().collection(USERS).requestOTP(u3.email);
    const m = await waitForMail((m) => m.to.includes(u3.email) && /OTP/.test(m.subject), { since });
    const err = await expectError(() => client().collection(USERS).authWithOTP(otpId, otpCode(m.html)));
    expect(err.status).toBe(401);
    expect(err.response.mfaId).toMatch(/^[a-z0-9]{15}$/);
    expect((await pb.collection("_mfas").getOne(err.response.mfaId)).method).toBe("otp");
  });

  test("mfa rule can exempt records", async () => {
    await pb.collections.update(usersId, { mfa: { enabled: true, duration: 600, rule: 'role = "manager"' } });
    try {
      const ok = await client().collection(USERS).authWithPassword(u1.email, PASS);
      expect(ok.record.id).toBe(u1.id);
      const err = await expectError(() => client().collection(USERS).authWithPassword(u2.email, PASS));
      expect(err.status).toBe(401);
      expect(err.response.mfaId).toBeDefined();
    } finally {
      await pb.collections.update(usersId, { mfa: { enabled: true, duration: 600, rule: "" } });
    }
  });
});
