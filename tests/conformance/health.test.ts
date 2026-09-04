import { describe, expect, test } from "bun:test";
import { BASE_URL, adminClient, client } from "./harness";

describe("health", () => {
  test("GET /api/health returns the PocketBase envelope", async () => {
    const pb = client();
    const res = await pb.health.check();
    // NOTE: health is the one endpoint that still uses `code` instead of `status`.
    expect(res.code).toBe(200);
    expect(res.message).toBe("API is healthy.");
    expect(res.data).toEqual({});
  });

  test("raw fetch: JSON body and content-type", async () => {
    const res = await fetch(`${BASE_URL}/api/health`);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toMatch(/^application\/json/);
    const body = await res.json();
    expect(Object.keys(body).sort()).toEqual(["code", "data", "message"]);
  });

  test("superuser sees canBackup / realIP / possibleProxyHeader in data", async () => {
    const pb = await adminClient();
    const res = await pb.health.check();
    expect(res.code).toBe(200);
    expect(res.data.canBackup).toBe(true);
    expect(typeof res.data.realIP).toBe("string");
    expect(res.data).toHaveProperty("possibleProxyHeader");
  });
});
