/** Thin typed wrappers over the superuser-only management endpoints —
 * everything the dashboard needs beyond plain record CRUD. Each group
 * mirrors one `crates/server/src/routes/*` module. */

import type { Transport } from "./transport.js";
import type { CollectionInput, CollectionModel, ListResult } from "./types.js";

type AuthHeader = () => string | undefined;

export class CollectionsAdmin {
  private readonly transport: Transport;
  private readonly authHeader: AuthHeader;
  constructor(transport: Transport, authHeader: AuthHeader) {
    this.transport = transport;
    this.authHeader = authHeader;
  }
  list(): Promise<{ items: CollectionModel[] }> {
    return this.transport.send("/api/collections", {}, this.authHeader);
  }
  one(idOrName: string): Promise<CollectionModel> {
    return this.transport.send(`/api/collections/${encodeURIComponent(idOrName)}`, {}, this.authHeader);
  }
  create(data: CollectionInput): Promise<CollectionModel> {
    return this.transport.send("/api/collections", { method: "POST", body: data }, this.authHeader);
  }
  update(idOrName: string, data: CollectionInput): Promise<CollectionModel> {
    return this.transport.send(
      `/api/collections/${encodeURIComponent(idOrName)}`,
      { method: "PATCH", body: data },
      this.authHeader,
    );
  }
  delete(idOrName: string): Promise<void> {
    return this.transport.send(
      `/api/collections/${encodeURIComponent(idOrName)}`,
      { method: "DELETE" },
      this.authHeader,
    );
  }
  truncate(idOrName: string): Promise<void> {
    return this.transport.send(
      `/api/collections/${encodeURIComponent(idOrName)}/truncate`,
      { method: "DELETE" },
      this.authHeader,
    );
  }
  import(collections: CollectionInput[], deleteMissing: boolean): Promise<void> {
    return this.transport.send(
      "/api/collections/import",
      { method: "PUT", body: { collections, deleteMissing } },
      this.authHeader,
    );
  }
  scaffolds(): Promise<Record<string, unknown>> {
    return this.transport.send("/api/collections/meta/scaffolds", {}, this.authHeader);
  }
}

export class SchemaAdmin {
  private readonly transport: Transport;
  private readonly authHeader: AuthHeader;
  constructor(transport: Transport, authHeader: AuthHeader) {
    this.transport = transport;
    this.authHeader = authHeader;
  }
  apply(
    collections: CollectionInput[],
    options: { dryRun?: boolean; force?: boolean } = {},
  ): Promise<Record<string, unknown>> {
    return this.transport.send(
      "/api/schema/apply",
      { method: "POST", body: { collections }, query: { dryRun: options.dryRun, force: options.force } },
      this.authHeader,
    );
  }
}

export class SettingsAdmin {
  private readonly transport: Transport;
  private readonly authHeader: AuthHeader;
  constructor(transport: Transport, authHeader: AuthHeader) {
    this.transport = transport;
    this.authHeader = authHeader;
  }
  get(): Promise<Record<string, unknown>> {
    return this.transport.send("/api/settings", {}, this.authHeader);
  }
  update(patch: Record<string, unknown>): Promise<Record<string, unknown>> {
    return this.transport.send("/api/settings", { method: "PATCH", body: patch }, this.authHeader);
  }
  testS3(which: "storage" | "backups" = "storage"): Promise<void> {
    return this.transport.send("/api/settings/test/s3", { method: "POST", body: { which } }, this.authHeader);
  }
  testEmail(collection: string, email: string, template: string): Promise<void> {
    return this.transport.send(
      "/api/settings/test/email",
      { method: "POST", body: { collection, email, template } },
      this.authHeader,
    );
  }
  generateAppleClientSecret(payload: Record<string, unknown>): Promise<{ secret: string }> {
    return this.transport.send(
      "/api/settings/apple/generate-client-secret",
      { method: "POST", body: payload },
      this.authHeader,
    );
  }
}

export interface LogEntry {
  id: string;
  message: string;
  level: number;
  created: string;
  data: Record<string, unknown>;
}

export class LogsAdmin {
  private readonly transport: Transport;
  private readonly authHeader: AuthHeader;
  constructor(transport: Transport, authHeader: AuthHeader) {
    this.transport = transport;
    this.authHeader = authHeader;
  }
  list(
    options: { page?: number; perPage?: number; filter?: string; sort?: string; skipTotal?: boolean } = {},
  ): Promise<ListResult<LogEntry>> {
    return this.transport.send("/api/logs", { query: options }, this.authHeader);
  }
  one(id: string): Promise<LogEntry> {
    return this.transport.send(`/api/logs/${encodeURIComponent(id)}`, {}, this.authHeader);
  }
  stats(filter?: string): Promise<unknown[]> {
    return this.transport.send("/api/logs/stats", { query: { filter } }, this.authHeader);
  }
}

export class BackupsAdmin {
  private readonly transport: Transport;
  private readonly authHeader: AuthHeader;
  constructor(transport: Transport, authHeader: AuthHeader) {
    this.transport = transport;
    this.authHeader = authHeader;
  }
  list(): Promise<Array<{ key: string; size: number; modified: string }>> {
    return this.transport.send("/api/backups", {}, this.authHeader);
  }
  create(): Promise<{ key: string }> {
    return this.transport.send("/api/backups", { method: "POST", body: {} }, this.authHeader);
  }
  upload(file: File | Blob): Promise<void> {
    const form = new FormData();
    form.append("file", file);
    return this.transport.send("/api/backups/upload", { method: "POST", body: form }, this.authHeader);
  }
  delete(key: string): Promise<void> {
    return this.transport.send(`/api/backups/${encodeURIComponent(key)}`, { method: "DELETE" }, this.authHeader);
  }
  restore(key: string): Promise<void> {
    return this.transport.send(
      `/api/backups/${encodeURIComponent(key)}/restore`,
      { method: "POST" },
      this.authHeader,
    );
  }
  storageInfo(): Promise<{ driver: "local" | "s3"; location: string }> {
    return this.transport.send("/api/backups/storage-info", {}, this.authHeader);
  }
  downloadURL(key: string, token: string): string {
    return `${this.transport.buildURL(`/api/backups/${encodeURIComponent(key)}`)}?token=${encodeURIComponent(token)}`;
  }
}

export class CronsAdmin {
  private readonly transport: Transport;
  private readonly authHeader: AuthHeader;
  constructor(transport: Transport, authHeader: AuthHeader) {
    this.transport = transport;
    this.authHeader = authHeader;
  }
  list(): Promise<Array<{ id: string; expression: string }>> {
    return this.transport.send("/api/crons", {}, this.authHeader);
  }
  run(id: string): Promise<void> {
    return this.transport.send(`/api/crons/${encodeURIComponent(id)}`, { method: "POST" }, this.authHeader);
  }
}

export class StorageAdmin {
  private readonly transport: Transport;
  private readonly authHeader: AuthHeader;
  constructor(transport: Transport, authHeader: AuthHeader) {
    this.transport = transport;
    this.authHeader = authHeader;
  }
  list(prefix?: string): Promise<{ files: Array<{ key: string; size: number }>; folders: string[] }> {
    return this.transport.send("/api/storage/objects", { query: { prefix } }, this.authHeader);
  }
  upload(key: string, file: File | Blob, filename: string): Promise<void> {
    const form = new FormData();
    form.append("key", key);
    form.append("file", file, filename);
    return this.transport.send("/api/storage/objects", { method: "POST", body: form }, this.authHeader);
  }
  delete(key: string): Promise<void> {
    return this.transport.send("/api/storage/objects", { method: "DELETE", query: { key } }, this.authHeader);
  }
  downloadURL(key: string): string {
    return `${this.transport.buildURL("/api/storage/objects/download")}?key=${encodeURIComponent(key)}`;
  }
}

export class ApiKeysAdmin {
  private readonly transport: Transport;
  private readonly authHeader: AuthHeader;
  constructor(transport: Transport, authHeader: AuthHeader) {
    this.transport = transport;
    this.authHeader = authHeader;
  }
  create(payload: {
    name: string;
    actsAsCollection?: string;
    actsAsRecord?: string;
  }): Promise<{ id: string; name: string; prefix: string; key: string }> {
    return this.transport.send("/api/api-keys", { method: "POST", body: payload }, this.authHeader);
  }
}

export class PushAdmin {
  private readonly transport: Transport;
  private readonly authHeader: AuthHeader;
  constructor(transport: Transport, authHeader: AuthHeader) {
    this.transport = transport;
    this.authHeader = authHeader;
  }
  send(payload: Record<string, unknown>): Promise<{ sent: number; failed: number }> {
    return this.transport.send("/api/push/send", { method: "POST", body: payload }, this.authHeader);
  }
}

export class AdminNamespace {
  readonly collections: CollectionsAdmin;
  readonly schema: SchemaAdmin;
  readonly settings: SettingsAdmin;
  readonly logs: LogsAdmin;
  readonly backups: BackupsAdmin;
  readonly crons: CronsAdmin;
  readonly storage: StorageAdmin;
  readonly apiKeys: ApiKeysAdmin;
  readonly push: PushAdmin;
  private readonly transport: Transport;
  private readonly authHeader: AuthHeader;

  constructor(transport: Transport, authHeader: AuthHeader) {
    this.transport = transport;
    this.authHeader = authHeader;
    this.collections = new CollectionsAdmin(transport, authHeader);
    this.schema = new SchemaAdmin(transport, authHeader);
    this.settings = new SettingsAdmin(transport, authHeader);
    this.logs = new LogsAdmin(transport, authHeader);
    this.backups = new BackupsAdmin(transport, authHeader);
    this.crons = new CronsAdmin(transport, authHeader);
    this.storage = new StorageAdmin(transport, authHeader);
    this.apiKeys = new ApiKeysAdmin(transport, authHeader);
    this.push = new PushAdmin(transport, authHeader);
  }

  sql(sql: string, write = false): Promise<Record<string, unknown>> {
    return this.transport.send("/api/sql", { method: "POST", body: { sql, write } }, this.authHeader);
  }

  functions(): Promise<{ hooksDir: string; files: unknown[]; routes: unknown[] }> {
    return this.transport.send("/api/functions", {}, this.authHeader);
  }

  health(): Promise<Record<string, unknown>> {
    return this.transport.send("/api/health", {}, this.authHeader);
  }

  setupStatus(): Promise<{ needsSetup: boolean }> {
    return this.transport.send("/api/setup/status", {}, this.authHeader);
  }

  setup(payload: { email: string; password: string; passwordConfirm: string }): Promise<void> {
    return this.transport.send("/api/setup", { method: "POST", body: payload }, this.authHeader);
  }
}
