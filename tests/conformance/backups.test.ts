import { beforeAll, describe, expect, test } from "bun:test";
import type PocketBase from "pocketbase";
import { adminClient, client, expectError, uniq } from "./harness";

/** Backups API: /api/backups (superuser only). */
let pb: PocketBase;

beforeAll(async () => {
  pb = await adminClient();
});

describe("backups", () => {
  test("create -> list -> download -> delete", async () => {
    const key = `${uniq("bk").replace(/_/g, "")}.zip`;
    expect(await pb.backups.create(key)).toBe(true);

    const list = await pb.backups.getFullList();
    expect(Array.isArray(list)).toBe(true);
    const mine = list.find((b) => b.key === key)!;
    expect(mine).toBeDefined();
    expect(Object.keys(mine).sort()).toEqual(["key", "modified", "size"]);
    expect(typeof mine.size).toBe("number");
    expect(mine.size).toBeGreaterThan(0);
    expect(mine.modified).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}Z$/);

    // download requires a superuser file token in the URL
    const token = await pb.files.getToken();
    const url = pb.backups.getDownloadURL(token, key);
    expect(url).toBe(`${pb.baseURL}/api/backups/${key}?token=${token}`);
    const res = await fetch(url);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("zip");
    const bytes = new Uint8Array(await res.arrayBuffer());
    expect(bytes.length).toBe(mine.size);
    expect(bytes[0]).toBe(0x50); // "PK" zip magic
    expect(bytes[1]).toBe(0x4b);
    // no token -> 403
    const noToken = await fetch(`${pb.baseURL}/api/backups/${key}`);
    expect(noToken.status).toBe(403);
    expect((await noToken.json()).message).toBe("Insufficient permissions to access the resource.");

    expect(await pb.backups.delete(key)).toBe(true);
    expect((await pb.backups.getFullList()).some((b) => b.key === key)).toBe(false);
  });

  test("auto-generated name when no key is given", async () => {
    expect(await pb.backups.create("")).toBe(true);
    const list = await pb.backups.getFullList();
    const auto = list.find((b) => /^pb_backup_/.test(b.key));
    expect(auto).toBeDefined();
    await pb.backups.delete(auto!.key);
  });

  test("invalid / duplicate key is rejected", async () => {
    const err = await expectError(() => pb.backups.create("../escape.zip"));
    expect(err.status).toBe(400);
    expect(err.response.data.name).toBeDefined();

    const key = `${uniq("dup").replace(/_/g, "")}.zip`;
    await pb.backups.create(key);
    try {
      const dup = await expectError(() => pb.backups.create(key));
      expect(dup.status).toBe(400);
      expect(dup.response.data.name.code).toBe("validation_backup_name_exists");
    } finally {
      await pb.backups.delete(key);
    }
  });

  test("deleting a missing backup -> 400", async () => {
    const err = await expectError(() => pb.backups.delete("does-not-exist.zip"));
    expect(err.status).toBe(400);
    expect(err.response.message).toContain("Invalid or already deleted backup file.");
  });

  test("restoring a missing backup -> 400", async () => {
    const err = await expectError(() => pb.backups.restore("does-not-exist.zip"));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Missing or invalid backup file.");
  });

  test("uploading a non-zip is rejected", async () => {
    const err = await expectError(() => pb.backups.upload({ file: new File(["not a zip"], "fake.zip", { type: "application/zip" }) }));
    expect(err.status).toBe(400);
    expect(err.response.data.file).toBeDefined();
  });

  test("backups are superuser-only", async () => {
    const err = await expectError(() => client().backups.getFullList());
    expect(err.status).toBe(401);
    const err2 = await expectError(() => client().backups.create("x.zip"));
    expect(err2.status).toBe(401);
  });

  // Restoring replaces pb_data and restarts the app, which would break every
  // other test in the run, so the happy path is intentionally not exercised.
  test.skip("restore replaces the data directory", () => {});
});
