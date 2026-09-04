import { beforeAll, describe, expect, test } from "bun:test";
import type PocketBase from "pocketbase";
import { adminClient, client, expectError } from "./harness";

/** Crons API: /api/crons (superuser only). */
let pb: PocketBase;

beforeAll(async () => {
  pb = await adminClient();
});

describe("crons", () => {
  test("getFullList returns the built-in jobs", async () => {
    const jobs = await pb.crons.getFullList();
    expect(Array.isArray(jobs)).toBe(true);
    for (const j of jobs) expect(Object.keys(j).sort()).toEqual(["expression", "id"]);
    const byId = Object.fromEntries(jobs.map((j) => [j.id, j.expression]));
    expect(byId["__pbDBOptimize__"]).toBe("0 0 * * *");
    expect(byId["__pbMFACleanup__"]).toBe("0 * * * *");
    expect(byId["__pbOTPCleanup__"]).toBe("0 * * * *");
    expect(byId["__pbLogsCleanup__"]).toBe("0 */6 * * *");
    expect(jobs.length).toBeGreaterThanOrEqual(4);
  });

  test("run executes a job synchronously (204)", async () => {
    expect(await pb.crons.run("__pbLogsCleanup__")).toBe(true);
    expect(await pb.crons.run("__pbOTPCleanup__")).toBe(true);
    expect(await pb.crons.run("__pbMFACleanup__")).toBe(true);
  });

  test("running an unknown job -> 404", async () => {
    const err = await expectError(() => pb.crons.run("__nope__"));
    expect(err.status).toBe(404);
    expect(err.response).toEqual({ status: 404, message: "Missing or invalid cron job.", data: {} });
  });

  test("crons are superuser-only", async () => {
    const err = await expectError(() => client().crons.getFullList());
    expect(err.status).toBe(401);
    expect(err.response.message).toBe("The request requires valid record authorization token.");
    const err2 = await expectError(() => client().crons.run("__pbLogsCleanup__"));
    expect(err2.status).toBe(401);
  });

  test("the backup cron appears once settings.backups.cron is set", async () => {
    const before = await pb.settings.getAll();
    await pb.settings.update({ backups: { ...before.backups, cron: "0 3 * * *" } });
    try {
      const jobs = await pb.crons.getFullList();
      const backup = jobs.find((j) => j.id === "__pbAutoBackup__");
      expect(backup).toBeDefined();
      expect(backup!.expression).toBe("0 3 * * *");
    } finally {
      await pb.settings.update({ backups: { ...before.backups, cron: before.backups.cron } });
    }
  });
});
