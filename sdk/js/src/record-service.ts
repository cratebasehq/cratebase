import type { Cratebase } from "./client.js";
import type { AuthResponse, ListOptions, ListResult, RecordModel } from "./types.js";

/**
 * CRUD + auth operations scoped to one collection. Auth-only methods
 * (`authWithPassword`, `authRefresh`) only make sense for `auth`-typed
 * collections but are exposed here unconditionally since the client has no
 * way to know a collection's type ahead of a request.
 */
export class RecordService<T extends RecordModel = RecordModel> {
  constructor(
    private readonly client: Cratebase,
    private readonly collectionIdOrName: string,
  ) {}

  private get basePath(): string {
    return `/api/collections/${this.collectionIdOrName}/records`;
  }

  async getList(page = 1, perPage = 30, options: ListOptions = {}): Promise<ListResult<T>> {
    return this.client.send<ListResult<T>>(this.basePath, {
      method: "GET",
      query: { page, perPage, filter: options.filter, sort: options.sort },
    });
  }

  /** Fetches every page and concatenates the results. Convenient for small
   * collections; prefer `getList` with pagination for large ones. */
  async getFullList(options: ListOptions & { batchSize?: number } = {}): Promise<T[]> {
    const batchSize = options.batchSize ?? 200;
    const items: T[] = [];
    let page = 1;
    for (;;) {
      const result = await this.getList(page, batchSize, options);
      items.push(...result.items);
      if (page >= result.totalPages) break;
      page += 1;
    }
    return items;
  }

  async getOne(id: string): Promise<T> {
    return this.client.send<T>(`${this.basePath}/${id}`, { method: "GET" });
  }

  async create(data: Record<string, unknown> | FormData): Promise<T> {
    return this.client.send<T>(this.basePath, { method: "POST", body: data });
  }

  async update(id: string, data: Record<string, unknown> | FormData): Promise<T> {
    return this.client.send<T>(`${this.basePath}/${id}`, { method: "PATCH", body: data });
  }

  async delete(id: string): Promise<void> {
    await this.client.send<void>(`${this.basePath}/${id}`, { method: "DELETE" });
  }

  async authWithPassword(email: string, password: string): Promise<AuthResponse<T>> {
    const result = await this.client.send<AuthResponse<T>>(`/api/collections/${this.collectionIdOrName}/auth-with-password`, {
      method: "POST",
      body: { email, password },
    });
    this.client.authStore.save(result.token, result.record);
    return result;
  }

  async authRefresh(): Promise<AuthResponse<T>> {
    const result = await this.client.send<AuthResponse<T>>(`/api/collections/${this.collectionIdOrName}/auth-refresh`, {
      method: "POST",
    });
    this.client.authStore.save(result.token, result.record);
    return result;
  }
}
