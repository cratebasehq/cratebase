/** `POST /api/batch`: several record writes in one HTTP round trip and
 * one database transaction. */

import type { Transport } from "./transport.js";

interface SubRequest {
  method: string;
  url: string;
  body?: Record<string, unknown>;
  files?: Record<string, File | Blob | Array<File | Blob>>;
}

export interface BatchResult {
  status: number;
  body: unknown;
}

function hasFile(value: Record<string, unknown> | undefined): boolean {
  if (!value) return false;
  return Object.values(value).some(
    (v) => v instanceof Blob || (Array.isArray(v) && v.some((item) => item instanceof Blob)),
  );
}

export class BatchBuilder {
  private readonly transport: Transport;
  private readonly authHeader: () => string | undefined;
  private readonly requests: SubRequest[] = [];

  constructor(transport: Transport, authHeader: () => string | undefined) {
    this.transport = transport;
    this.authHeader = authHeader;
  }

  create(collection: string, data: Record<string, unknown>): this {
    this.push("POST", `/api/collections/${encodeURIComponent(collection)}/records`, data);
    return this;
  }

  update(collection: string, id: string, data: Record<string, unknown>): this {
    this.push(
      "PATCH",
      `/api/collections/${encodeURIComponent(collection)}/records/${encodeURIComponent(id)}`,
      data,
    );
    return this;
  }

  delete(collection: string, id: string): this {
    this.push(
      "DELETE",
      `/api/collections/${encodeURIComponent(collection)}/records/${encodeURIComponent(id)}`,
    );
    return this;
  }

  /** `data.id` present → `update`; absent → `create`. Batch has no
   * native upsert endpoint — this is the same "id decides" convention
   * every other upsert-flavored helper in the ecosystem uses. */
  upsert(collection: string, data: Record<string, unknown> & { id?: string }): this {
    const { id, ...rest } = data;
    if (id) return this.update(collection, id, rest);
    return this.create(collection, rest);
  }

  private push(method: string, url: string, data?: Record<string, unknown>): void {
    if (!data) {
      this.requests.push({ method, url });
      return;
    }
    const files: Record<string, File | Blob | Array<File | Blob>> = {};
    const body: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(data)) {
      if (value instanceof Blob || (Array.isArray(value) && value.some((v) => v instanceof Blob))) {
        files[key] = value as File | Blob | Array<File | Blob>;
      } else {
        body[key] = value;
      }
    }
    this.requests.push({ method, url, body, files: Object.keys(files).length > 0 ? files : undefined });
  }

  async send(): Promise<BatchResult[]> {
    const anyFiles = this.requests.some((r) => r.files);
    if (!anyFiles) {
      return this.transport.send<BatchResult[]>(
        "/api/batch",
        {
          method: "POST",
          body: {
            requests: this.requests.map((r) => ({ method: r.method, url: r.url, body: r.body ?? {} })),
          },
        },
        this.authHeader,
      );
    }

    const form = new FormData();
    form.append(
      "@jsonPayload",
      JSON.stringify({
        requests: this.requests.map((r) => ({ method: r.method, url: r.url, body: r.body ?? {} })),
      }),
    );
    this.requests.forEach((r, index) => {
      if (!hasFile(r.files as Record<string, unknown> | undefined)) return;
      for (const [field, value] of Object.entries(r.files ?? {})) {
        const values = Array.isArray(value) ? value : [value];
        for (const v of values) form.append(`requests.${index}.${field}`, v);
      }
    });
    return this.transport.send<BatchResult[]>(
      "/api/batch",
      { method: "POST", body: form },
      this.authHeader,
    );
  }
}
