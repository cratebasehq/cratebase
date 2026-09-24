/** A minimal fake `CratebaseClient`, hand-rolled rather than importing
 * `@cratebase/client`'s real `CratebaseClient`/`CollectionService`: every
 * hook in this package only ever calls a handful of methods
 * (`collection(name).{list,fullList,one,create,update,delete,subscribe}`,
 * `auth.{record,token,isValid,isSuperuser,onChange,signIn,signOut}`,
 * `presence.track`), so a fake that implements exactly that surface, with
 * an in-memory store and a synchronous fake pub/sub the test can drive
 * directly (`emit`), is both simpler and faster than spinning up the real
 * client against a real or mocked HTTP transport. */

import type { RecordModel } from "@cratebase/client";

export interface FakeEvent<T> {
  action: "create" | "update" | "delete";
  record: T;
}

type Handler<T> = (event: FakeEvent<T>) => void;

export interface FakeCollection<T extends RecordModel> {
  list(options?: any): Promise<{ page: number; perPage: number; totalItems: number; totalPages: number; items: T[] }>;
  fullList(options?: any): Promise<T[]>;
  one(id: string, options?: any): Promise<T>;
  create(data: any): Promise<T>;
  update(id: string, data: any): Promise<T>;
  delete(id: string): Promise<void>;
  subscribe(topic: string, handler: Handler<T>, options?: any): Promise<() => void>;
  /** Test-only: pushes a fake realtime event to every listener currently
   * subscribed to `topic` (`"*"` or a specific record id), synchronously. */
  emit(topic: string, event: FakeEvent<T>): void;
  /** Test-only: current subscriber count, for asserting cleanup. */
  listenerCount(): number;
}

export function createFakeCollection<T extends RecordModel>(seed: T[] = []): FakeCollection<T> {
  let rows = [...seed];
  const listeners = new Map<string, Set<Handler<T>>>();
  let nextId = seed.length + 1;

  function matches(item: T, filter?: string): boolean {
    // Only the trivial patterns this test suite actually uses:
    // no filter, or `field = "value"` / `field = true` / `field = false`.
    if (!filter) return true;
    const m = /^(\w+)\s*=\s*(true|false|"([^"]*)")$/.exec(filter.trim());
    if (!m) return true;
    const [, field, literal, quoted] = m;
    const expected = literal === "true" ? true : literal === "false" ? false : quoted;
    return (item as any)[field!] === expected;
  }

  function sortItems(items: T[], sort?: string): T[] {
    if (!sort) return items;
    const desc = sort.startsWith("-");
    const field = desc ? sort.slice(1) : sort.replace(/^\+/, "");
    return [...items].sort((a, b) => {
      const av = (a as any)[field];
      const bv = (b as any)[field];
      const cmp = av < bv ? -1 : av > bv ? 1 : 0;
      return desc ? -cmp : cmp;
    });
  }

  function query(options: any = {}): T[] {
    return sortItems(rows.filter((r) => matches(r, options.filter)), options.sort);
  }

  return {
    async list(options: any = {}) {
      const filtered = query(options);
      const page = options.page ?? 1;
      const perPage = options.perPage ?? 30;
      const start = (page - 1) * perPage;
      const items = filtered.slice(start, start + perPage);
      return {
        page,
        perPage,
        totalItems: filtered.length,
        totalPages: Math.max(1, Math.ceil(filtered.length / perPage)),
        items,
      };
    },
    async fullList(options: any = {}) {
      return query(options);
    },
    async one(id: string) {
      const found = rows.find((r) => r.id === id);
      if (!found) throw new Error(`not found: ${id}`);
      return found;
    },
    async create(data: any) {
      const record = { id: `rec_${nextId++}`, collectionId: "c1", collectionName: "test", ...data } as T;
      rows = [...rows, record];
      dispatch("*", { action: "create", record });
      return record;
    },
    async update(id: string, data: any) {
      const idx = rows.findIndex((r) => r.id === id);
      if (idx === -1) throw new Error(`not found: ${id}`);
      const record = { ...rows[idx], ...data } as T;
      rows = [...rows.slice(0, idx), record, ...rows.slice(idx + 1)];
      dispatch("*", { action: "update", record });
      dispatch(id, { action: "update", record });
      return record;
    },
    async delete(id: string) {
      const idx = rows.findIndex((r) => r.id === id);
      if (idx === -1) throw new Error(`not found: ${id}`);
      const record = rows[idx]!;
      rows = [...rows.slice(0, idx), ...rows.slice(idx + 1)];
      dispatch("*", { action: "delete", record });
      dispatch(id, { action: "delete", record });
    },
    async subscribe(topic: string, handler: Handler<T>) {
      let set = listeners.get(topic);
      if (!set) {
        set = new Set();
        listeners.set(topic, set);
      }
      set.add(handler);
      return () => {
        set!.delete(handler);
      };
    },
    emit(topic: string, event: FakeEvent<T>) {
      dispatch(topic, event);
    },
    listenerCount() {
      let n = 0;
      for (const set of listeners.values()) n += set.size;
      return n;
    },
  };

  function dispatch(topic: string, event: FakeEvent<T>) {
    const set = listeners.get(topic);
    if (!set) return;
    for (const handler of [...set]) handler(event);
  }
}

export interface FakeAuth {
  record: RecordModel | null;
  token: string;
  isValid: boolean;
  isSuperuser: boolean;
  onChange(cb: (token: string, record: RecordModel | null) => void): () => void;
  signIn: { password: (opts: any) => Promise<any> };
  signOut(): Promise<void>;
  /** Test-only: signs a record in and notifies listeners. */
  _signInAs(record: RecordModel, token?: string): void;
}

export function createFakeAuth(): FakeAuth {
  let record: RecordModel | null = null;
  let token = "";
  const listeners = new Set<(token: string, record: RecordModel | null) => void>();

  const auth: FakeAuth = {
    get record() {
      return record;
    },
    get token() {
      return token;
    },
    get isValid() {
      return Boolean(token);
    },
    get isSuperuser() {
      return record?.collectionName === "_superusers";
    },
    onChange(cb) {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    signIn: {
      async password(opts: any) {
        record = { id: "user_1", collectionId: "users", collectionName: "users", email: opts.identity };
        token = "tok_1";
        for (const l of listeners) l(token, record);
        return { token, record };
      },
    },
    async signOut() {
      record = null;
      token = "";
      for (const l of listeners) l(token, record);
    },
    _signInAs(r, t = "tok_1") {
      record = r;
      token = t;
      for (const l of listeners) l(token, record);
    },
  };
  return auth;
}

export interface FakePresenceHandle {
  online: Set<string>;
  subscribe(cb: (online: Set<string>) => void): () => void;
  stop(): Promise<void>;
}

export function createFakeClient(collections: Record<string, FakeCollection<any>> = {}) {
  const auth = createFakeAuth();
  const collectionMap = new Map<string, FakeCollection<any>>(Object.entries(collections));

  const presenceHandles: FakePresenceHandle[] = [];

  return {
    auth,
    collection(name: string): FakeCollection<any> {
      let c = collectionMap.get(name);
      if (!c) {
        c = createFakeCollection([]);
        collectionMap.set(name, c);
      }
      return c;
    },
    presence: {
      async track(_collectionName: string, data: Record<string, unknown> & { id?: string }): Promise<FakePresenceHandle> {
        const listeners = new Set<(online: Set<string>) => void>();
        let online = new Set<string>([data.id ?? "self"]);
        const handle: FakePresenceHandle = {
          get online() {
            return online;
          },
          subscribe(cb) {
            listeners.add(cb);
            cb(online);
            return () => listeners.delete(cb);
          },
          async stop() {
            listeners.clear();
          },
        };
        presenceHandles.push(handle);
        return handle;
      },
    },
    _presenceHandles: presenceHandles,
  } as any;
}
