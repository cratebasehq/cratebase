import type { Cratebase } from "./client.js";
import type { CollectionModel } from "./types.js";

/** Manage collection schemas. Requires a superuser token in
 * `client.authStore`. */
export class SchemaService {
  constructor(private readonly client: Cratebase) {}

  async getList(): Promise<CollectionModel[]> {
    return this.client.send<CollectionModel[]>("/api/collections", { method: "GET" });
  }

  async getOne(idOrName: string): Promise<CollectionModel> {
    return this.client.send<CollectionModel>(`/api/collections/${idOrName}`, { method: "GET" });
  }

  async create(data: Omit<CollectionModel, "id" | "created" | "updated">): Promise<CollectionModel> {
    return this.client.send<CollectionModel>("/api/collections", { method: "POST", body: data });
  }

  async update(idOrName: string, data: Omit<CollectionModel, "id" | "created" | "updated">): Promise<CollectionModel> {
    return this.client.send<CollectionModel>(`/api/collections/${idOrName}`, { method: "PATCH", body: data });
  }

  async delete(idOrName: string): Promise<void> {
    await this.client.send<void>(`/api/collections/${idOrName}`, { method: "DELETE" });
  }
}
