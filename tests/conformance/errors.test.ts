import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import type PocketBase from "pocketbase";
import { ClientResponseError, adminClient, client, dropCollection, expectError, uniq } from "./harness";

/**
 * Error envelope: every API error is `{status, message, data}` (v0.23 renamed
 * `code` to `status`; only /api/health still uses `code`).
 */
let pb: PocketBase;
const LOCKED = uniq("locked");
const OPEN = uniq("open");

beforeAll(async () => {
  pb = await adminClient();
  await pb.collections.create({ name: LOCKED, type: "base", fields: [{ name: "title", type: "text" }] });
  await pb.collections.create({
    name: OPEN,
    type: "base",
    listRule: "",
    viewRule: "",
    createRule: null,
    fields: [{ name: "title", type: "text", required: true }],
  });
  await pb.collection(LOCKED).create({ title: "hidden" });
  await pb.collection(OPEN).create({ title: "public" });
});

afterAll(async () => {
  await dropCollection(pb, LOCKED);
  await dropCollection(pb, OPEN);
});

describe("errors: envelope", () => {
  test("404 unknown collection", async () => {
    const err = await expectError(() => client().collection("does_not_exist_xyz").getList());
    expect(err.status).toBe(404);
    expect(err.response).toEqual({ status: 404, message: "Missing collection context.", data: {} });
    // the same shape over raw HTTP
    const res = await fetch(`${pb.baseURL}/api/collections/does_not_exist_xyz/records`);
    expect(res.status).toBe(404);
    expect(await res.json()).toEqual({ status: 404, message: "Missing collection context.", data: {} });
  });

  test("404 unknown record vs unknown route", async () => {
    const err = await expectError(() => client().collection(OPEN).getOne("nonexistent0001"));
    expect(err.response).toEqual({ status: 404, message: "The requested resource wasn't found.", data: {} });
    const res = await fetch(`${pb.baseURL}/api/not-a-route`);
    expect(res.status).toBe(404);
    const body = await res.json();
    expect(body.status).toBe(404);
    expect(body.data).toEqual({});
  });

  test("403 on a superuser-only (null) rule; 401 without a token on admin APIs", async () => {
    const err = await expectError(() => client().collection(LOCKED).getList());
    expect(err.status).toBe(403);
    expect(err.response).toEqual({ status: 403, message: "Only superusers can perform this action.", data: {} });
    // create with a null createRule fails the same way
    const err2 = await expectError(() => client().collection(OPEN).create({ title: "x" }));
    expect(err2.status).toBe(403);
    expect(err2.response.message).toBe("Only superusers can perform this action.");
    // admin-only endpoints require a token first
    const err3 = await expectError(() => client().collections.getList());
    expect(err3.status).toBe(401);
    expect(err3.response).toEqual({ status: 401, message: "The request requires valid record authorization token.", data: {} });
  });

  test("400 invalid filter / sort", async () => {
    const err = await expectError(() => client().collection(OPEN).getList(1, 1, { filter: "title = " }));
    expect(err.status).toBe(400);
    expect(err.response).toEqual({ status: 400, message: "Something went wrong while processing your request.", data: {} });
    const err2 = await expectError(() => client().collection(OPEN).getList(1, 1, { sort: "unknown_field" }));
    expect(err2.status).toBe(400);
    expect(err2.response.message).toBe("Something went wrong while processing your request.");
  });

  test("400 validation errors are keyed per field", async () => {
    const err = await expectError(() => pb.collection(OPEN).create({}));
    expect(err.status).toBe(400);
    expect(err.response.message).toBe("Failed to create record.");
    expect(err.response.data).toEqual({ title: { code: "validation_required", message: "Cannot be blank." } });
  });

  test("a wrong method on an existing route is a 404, not a 405", async () => {
    const res = await fetch(`${pb.baseURL}/api/health`, { method: "DELETE" });
    expect(res.status).toBe(404);
    const body = await res.json();
    expect(body.status).toBe(404);
    expect(body.data).toEqual({});
  });

  test("ClientResponseError fields", async () => {
    const c = client();
    const err = await expectError(() => c.collection("does_not_exist_xyz").getList());
    expect(err).toBeInstanceOf(ClientResponseError);
    expect(err).toBeInstanceOf(Error);
    expect(err.name).toBe("ClientResponseError 404");
    expect(err.status).toBe(404);
    expect(err.url).toBe(`${pb.baseURL}/api/collections/does_not_exist_xyz/records?page=1&perPage=30`);
    expect(err.message).toBe("Missing collection context.");
    expect(err.isAbort).toBe(false);
    expect(err.response.status).toBe(404);
    expect(err.response.data).toEqual({});
    // `data` is a shortcut for the whole response body
    expect(err.data).toEqual(err.response);
  });

  test("network failure surfaces as a ClientResponseError with status 0", async () => {
    const dead = new (await import("pocketbase")).default("http://127.0.0.1:1");
    dead.autoCancellation(false);
    const err = await expectError(() => dead.health.check());
    expect(err.status).toBe(0);
    expect(err.isAbort).toBe(false);
    expect(err.originalError).toBeDefined();
  });

  test("auto-cancellation raises an abort error", async () => {
    const c = client();
    c.autoCancellation(true);
    const first = c.collection(OPEN).getList(1, 1);
    const second = c.collection(OPEN).getList(1, 1);
    const err = await expectError(() => first);
    expect(err.isAbort).toBe(true);
    expect(err.status).toBe(0);
    await second;
  });
});
