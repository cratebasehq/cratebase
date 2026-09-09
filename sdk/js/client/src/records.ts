/** Typed CRUD over one collection: `GET/POST/PATCH/DELETE
 * /api/collections/{name}/records[/{id}]`. Object-options only — no
 * positional `getList(page, perPage, opts)` alias — so there is exactly
 * one calling convention in this codebase. */

import type { Transport } from "./transport.js";
import type { ListOptions, ListResult, RecordModel, SubscribeOptions, ViewOptions, WriteOptions } from "./types.js";
import type { RealtimeClient } from "./realtime.js";

/** Records paginate at 500/page internally when walking every page for
 * `fullList`. */
const FULL_LIST_PAGE_SIZE = 500;

export class CollectionService<T extends Record<string, unknown> = RecordModel> {
  private readonly transport: Transport;
  private readonly authHeader: () => string | undefined;
  private readonly realtime: RealtimeClient;
  readonly name: string;

  constructor(
    transport: Transport,
    authHeader: () => string | undefined,
    realtime: RealtimeClient,
    name: string,
  ) {
    this.transport = transport;
    this.authHeader = authHeader;
    this.realtime = realtime;
    this.name = name;
  }

  private basePath(suffix = ""): string {
    return `/api/collections/${encodeURIComponent(this.name)}/records${suffix}`;
  }

  async list(options: ListOptions<T> = {}): Promise<ListResult<T & RecordModel>> {
    return this.transport.send<ListResult<T & RecordModel>>(
      this.basePath(),
      {
        query: {
          page: options.page,
          perPage: options.perPage,
          sort: options.sort,
          filter: options.filter,
          expand: options.expand,
          fields: options.fields,
          skipTotal: options.skipTotal,
        },
        signal: options.signal,
      },
      this.authHeader,
    );
  }

  async fullList(options: Omit<ListOptions<T>, "page" | "perPage"> = {}): Promise<Array<T & RecordModel>> {
    const items: Array<T & RecordModel> = [];
    let page = 1;
    for (;;) {
      const result = await this.list({ ...options, page, perPage: FULL_LIST_PAGE_SIZE, skipTotal: true });
      items.push(...result.items);
      if (result.items.length < FULL_LIST_PAGE_SIZE) break;
      page += 1;
    }
    return items;
  }

  async first(options: Omit<ListOptions<T>, "page" | "perPage"> = {}): Promise<(T & RecordModel) | null> {
    const result = await this.list({ ...options, page: 1, perPage: 1, skipTotal: true });
    return result.items[0] ?? null;
  }

  async one(id: string, options: ViewOptions = {}): Promise<T & RecordModel> {
    return this.transport.send<T & RecordModel>(
      this.basePath(`/${encodeURIComponent(id)}`),
      { query: { expand: options.expand, fields: options.fields }, signal: options.signal },
      this.authHeader,
    );
  }

  async create(data: Partial<T> | FormData, options: WriteOptions = {}): Promise<T & RecordModel> {
    return this.transport.send<T & RecordModel>(
      this.basePath(),
      {
        method: "POST",
        body: data,
        query: { expand: options.expand, fields: options.fields },
        signal: options.signal,
      },
      this.authHeader,
    );
  }

  async update(id: string, data: Partial<T> | FormData, options: WriteOptions = {}): Promise<T & RecordModel> {
    return this.transport.send<T & RecordModel>(
      this.basePath(`/${encodeURIComponent(id)}`),
      {
        method: "PATCH",
        body: data,
        query: { expand: options.expand, fields: options.fields },
        signal: options.signal,
      },
      this.authHeader,
    );
  }

  async delete(id: string): Promise<void> {
    await this.transport.send<void>(
      this.basePath(`/${encodeURIComponent(id)}`),
      { method: "DELETE" },
      this.authHeader,
    );
  }

  /** `topic` is `"*"` (every record), a bare record id, or a
   * `?options=` suffix built from `SubscribeOptions` — matching
   * `crates/server/src/realtime.rs`'s topic grammar. */
  async subscribe(
    topic: string,
    handler: (event: { action: "create" | "update" | "delete"; record: T & RecordModel }) => void,
    options: SubscribeOptions = {},
  ): Promise<() => void> {
    return this.realtime.subscribe(this.name, topic, handler as (event: unknown) => void, options);
  }
}
