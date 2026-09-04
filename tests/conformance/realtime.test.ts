import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import type PocketBase from "pocketbase";
import type { RecordModel, RecordSubscription } from "pocketbase";
import { adminClient, client, dropCollection, uniq, waitFor } from "./harness";

/**
 * Realtime (SSE): /api/realtime.
 *
 * Bun provides a global EventSource, which the SDK picks up, so no polyfill is
 * needed. If you run this suite on a runtime without EventSource, install the
 * `eventsource` package and assign it to globalThis.EventSource before the SDK
 * is imported.
 */
let pb: PocketBase;
const POSTS = uniq("rtposts");
const AUTHORS = uniq("rtauthors");
const USERS = uniq("rtusers");
const PRIVATE = uniq("rtprivate");
let postsId = "";
let alice: RecordModel;

type Ev = RecordSubscription<RecordModel>;

/** Collect events into an array; returns the array + the unsubscribe fn. */
async function collect(
  c: PocketBase,
  collection: string,
  topic = "*",
  options: Record<string, unknown> = {},
): Promise<{ events: Ev[]; stop: () => Promise<void> }> {
  const events: Ev[] = [];
  const unsub = await c.collection(collection).subscribe(topic, (e) => events.push(e), options);
  // give the server a moment to register the subscription
  await Bun.sleep(150);
  return { events, stop: unsub };
}

beforeAll(async () => {
  pb = await adminClient();
  const authors = await pb.collections.create({
    name: AUTHORS,
    type: "base",
    listRule: "",
    viewRule: "",
    fields: [{ name: "name", type: "text" }],
  });
  const posts = await pb.collections.create({
    name: POSTS,
    type: "base",
    listRule: "",
    viewRule: "",
    createRule: "",
    updateRule: "",
    deleteRule: "",
    fields: [
      { name: "title", type: "text" },
      { name: "views", type: "number" },
      { name: "author", type: "relation", collectionId: authors.id, maxSelect: 1 },
      { name: "created", type: "autodate", onCreate: true, onUpdate: false },
    ],
  });
  postsId = posts.id;
  await pb.collections.create({ name: USERS, type: "auth", fields: [] });
  await pb.collections.create({
    name: PRIVATE,
    type: "base",
    fields: [{ name: "title", type: "text" }],
    listRule: `@request.auth.collectionName = "${USERS}"`,
    viewRule: `@request.auth.collectionName = "${USERS}"`,
  });
  alice = await pb.collection(AUTHORS).create({ name: "Alice" });
});

afterAll(async () => {
  for (const n of [POSTS, AUTHORS, PRIVATE, USERS]) await dropCollection(pb, n);
});

describe("realtime", () => {
  test("collection topic receives create / update / delete", async () => {
    const c = client();
    const { events, stop } = await collect(c, POSTS);
    const r = await pb.collection(POSTS).create({ title: "RT one", views: 1 });
    await pb.collection(POSTS).update(r.id, { views: 2 });
    await pb.collection(POSTS).delete(r.id);
    await waitFor(() => events.length >= 3);
    await stop();

    expect(events.map((e) => e.action)).toEqual(["create", "update", "delete"]);
    expect(events[0]!.record.id).toBe(r.id);
    expect(events[0]!.record.collectionId).toBe(postsId);
    expect(events[0]!.record.collectionName).toBe(POSTS);
    expect(events[0]!.record.title).toBe("RT one");
    expect(events[0]!.record.views).toBe(1);
    expect(events[1]!.record.views).toBe(2);
    expect(events[2]!.record.id).toBe(r.id);
    expect(Object.keys(events[0]!).sort()).toEqual(["action", "record"]);
  });

  test("record topic only receives events for that record", async () => {
    const a = await pb.collection(POSTS).create({ title: "Watched" });
    const b = await pb.collection(POSTS).create({ title: "Ignored" });
    const c = client();
    const { events, stop } = await collect(c, POSTS, a.id);
    await pb.collection(POSTS).update(b.id, { title: "Ignored 2" });
    await pb.collection(POSTS).update(a.id, { title: "Watched 2" });
    await waitFor(() => events.length >= 1);
    await Bun.sleep(200);
    await stop();
    expect(events).toHaveLength(1);
    expect(events[0]!.action).toBe("update");
    expect(events[0]!.record.id).toBe(a.id);
    await pb.collection(POSTS).delete(a.id);
    await pb.collection(POSTS).delete(b.id);
  });

  test("topic options: filter, fields and expand", async () => {
    const c = client();
    const { events, stop } = await collect(c, POSTS, "*", {
      filter: "views > 10",
      fields: "id,title,expand.author.name",
      expand: "author",
    });
    const skipped = await pb.collection(POSTS).create({ title: "Low", views: 1, author: alice.id });
    const matched = await pb.collection(POSTS).create({ title: "High", views: 42, author: alice.id });
    await waitFor(() => events.length >= 1);
    await Bun.sleep(200);
    await stop();

    expect(events).toHaveLength(1);
    const rec = events[0]!.record;
    expect(rec.id).toBe(matched.id);
    // `fields` trims the payload (collectionId/collectionName are dropped too)
    expect(Object.keys(rec).sort()).toEqual(["expand", "id", "title"]);
    expect(rec.expand?.author).toEqual({ name: "Alice" });
    await pb.collection(POSTS).delete(skipped.id);
    await pb.collection(POSTS).delete(matched.id);
  });

  test("listRule is enforced: a client without permission receives nothing", async () => {
    const anon = client();
    const { events: anonEvents, stop: stopAnon } = await collect(anon, PRIVATE);

    const user = client();
    const u = await pb.collection(USERS).create({ email: `rt@${USERS}.test`, password: "Password123!", passwordConfirm: "Password123!" });
    await user.collection(USERS).authWithPassword(u.email, "Password123!");
    const { events: userEvents, stop: stopUser } = await collect(user, PRIVATE);

    const r = await pb.collection(PRIVATE).create({ title: "secret" });
    await waitFor(() => userEvents.length >= 1);
    await Bun.sleep(300);
    await stopAnon();
    await stopUser();

    expect(userEvents.map((e) => e.action)).toEqual(["create"]);
    expect(userEvents[0]!.record.title).toBe("secret");
    expect(anonEvents).toEqual([]);
    await pb.collection(PRIVATE).delete(r.id);
  });

  test("unsubscribe stops delivery; multiple listeners on one client both fire", async () => {
    const c = client();
    const a: Ev[] = [];
    const b: Ev[] = [];
    const unsubA = await c.collection(POSTS).subscribe("*", (e) => a.push(e));
    const unsubB = await c.collection(POSTS).subscribe("*", (e) => b.push(e));
    await Bun.sleep(150);
    const r = await pb.collection(POSTS).create({ title: "Both" });
    await waitFor(() => a.length >= 1 && b.length >= 1);
    await unsubA();
    await Bun.sleep(150);
    await pb.collection(POSTS).update(r.id, { title: "Only B" });
    await waitFor(() => b.length >= 2);
    await Bun.sleep(150);
    expect(a).toHaveLength(1);
    expect(b).toHaveLength(2);
    await unsubB();
    await Bun.sleep(150);
    await pb.collection(POSTS).update(r.id, { title: "Nobody" });
    await Bun.sleep(300);
    expect(b).toHaveLength(2);
    await pb.collection(POSTS).delete(r.id);
  });

  test("unsubscribing everything closes the SSE connection", async () => {
    const c = client();
    const unsub = await c.collection(POSTS).subscribe("*", () => {});
    await Bun.sleep(150);
    expect(c.realtime.isConnected).toBe(true);
    // the realtime client id is a 40-char security.RandomString, not a record id
    expect(c.realtime.clientId).toMatch(/^[A-Za-z0-9]{40}$/);
    await unsub();
    await waitFor(() => !c.realtime.isConnected);
    expect(c.realtime.isConnected).toBe(false);
  });

  test("subscribing to an unknown collection reports an error on the first event", async () => {
    const c = client();
    const events: Ev[] = [];
    await c.collection("does_not_exist_rt").subscribe("*", (e) => events.push(e));
    await Bun.sleep(200);
    // no events, no crash; the subscription is simply never matched
    expect(events).toEqual([]);
    c.realtime.unsubscribe();
    await Bun.sleep(100);
  });
});
