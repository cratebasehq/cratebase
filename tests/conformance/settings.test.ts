import { beforeAll, describe, expect, test } from "bun:test";
import type PocketBase from "pocketbase";
import { adminClient, client, expectError } from "./harness";

/** Settings API: /api/settings (superuser only). */
let pb: PocketBase;

beforeAll(async () => {
  pb = await adminClient();
});

describe("settings", () => {
  test("getAll shape", async () => {
    const s = await pb.settings.getAll();
    expect(Object.keys(s).sort()).toEqual([
      "backups",
      "batch",
      "llm",
      "logs",
      "meta",
      "rateLimits",
      "s3",
      "smtp",
      "superuserIPs",
      "trustedProxy",
    ]);
    expect(Object.keys(s.meta).sort()).toEqual([
      "accentColor",
      "appName",
      "appURL",
      "hideControls",
      "senderAddress",
      "senderName",
    ]);
    expect(typeof s.meta.appName).toBe("string");
    expect(s.logs).toMatchObject({ maxDays: expect.any(Number), minLevel: expect.any(Number), logIP: expect.any(Boolean), logAuthId: expect.any(Boolean) });
    expect(s.batch).toMatchObject({ enabled: expect.any(Boolean), maxRequests: expect.any(Number), timeout: expect.any(Number) });
    expect(s.backups).toMatchObject({ cron: expect.any(String), cronMaxKeep: expect.any(Number) });
    expect(Object.keys(s.backups.s3).sort()).toEqual(["accessKey", "bucket", "enabled", "endpoint", "forcePathStyle", "region"]);
    expect(s.rateLimits).toMatchObject({ enabled: expect.any(Boolean), rules: expect.any(Array), excludedIPs: expect.any(Array) });
    expect(s.trustedProxy).toMatchObject({ headers: expect.any(Array), useLeftmostIP: expect.any(Boolean) });
    expect(s.superuserIPs).toEqual(expect.any(Array));
  });

  test("secrets are never returned", async () => {
    const s = await pb.settings.getAll();
    expect(s.smtp).not.toHaveProperty("password");
    expect(s.s3).not.toHaveProperty("secret");
    expect(s.backups.s3).not.toHaveProperty("secret");
    expect(s).not.toHaveProperty("secret");
    // the raw JSON has no secret-looking keys either
    const raw = await fetch(`${pb.baseURL}/api/settings`, { headers: { authorization: pb.authStore.token } });
    const text = await raw.text();
    expect(text).not.toContain('"secret"');
    expect(text).not.toContain('"password"');
  });

  test("update round trip on meta.appName (partial updates keep other keys)", async () => {
    const before = await pb.settings.getAll();
    const updated = await pb.settings.update({ meta: { ...before.meta, appName: "Conformance Renamed" } });
    expect(updated.meta.appName).toBe("Conformance Renamed");
    expect(updated.meta.senderAddress).toBe(before.meta.senderAddress);
    expect((await pb.settings.getAll()).meta.appName).toBe("Conformance Renamed");
    const restored = await pb.settings.update({ meta: before.meta });
    expect(restored.meta.appName).toBe(before.meta.appName);
  });

  test("update validation: bad appName / bad rate limit rule", async () => {
    const before = await pb.settings.getAll();
    let err = await expectError(() => pb.settings.update({ meta: { ...before.meta, appName: "" } }));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("An error occurred while saving the new settings.");
    expect(err.response.data.meta.appName.code).toBe("validation_required");
    err = await expectError(() => pb.settings.update({ meta: { ...before.meta, appURL: "not a url" } }));
    expect(err.response.data.meta.appURL).toBeDefined();
    err = await expectError(() => pb.settings.update({ logs: { ...before.logs, maxDays: -1 } }));
    expect(err.response.data.logs.maxDays).toBeDefined();
  });

  test("only superusers can read or write settings", async () => {
    const err = await expectError(() => client().settings.getAll());
    expect(err.status).toBe(401);
    expect(err.response.message).toBe("The request requires valid record authorization token.");
    const err2 = await expectError(() => client().settings.update({ meta: { appName: "hax" } }));
    expect(err2.status).toBe(401);
  });

  test("testS3 fails when s3 is disabled", async () => {
    const err = await expectError(() => pb.settings.testS3("storage"));
    expect(err.status).toBe(400);
    expect(err.response.message).toContain("Failed to test the S3 filesystem.");
    expect(err.response.message).toContain("S3 storage filesystem is not enabled.");
  });

  test("testEmail validates its arguments", async () => {
    const err = await expectError(() => pb.settings.testEmail("_superusers", "not-an-email", "verification"));
    expect(err.status).toBe(400);
    expect(err.response.data.email.code).toBe("validation_is_email");
    const err2 = await expectError(() => pb.settings.testEmail("_superusers", "a@b.c", "bogus-template" as never));
    expect(err2.status).toBe(400);
    expect(err2.response.data.template).toBeDefined();
  });
});
