/** `/api/files/*`: building a file's URL and minting protected-file
 * tokens. */

import type { Transport } from "./transport.js";
import type { RecordModel } from "./types.js";

export interface FileURLOptions {
  /** `WxH`, `WxHt|b|f` (crop to top/bottom/fit), `0xH`, or `Wx0` — the
   * thumb grammar `crates/server/src/routes/files.rs` accepts. Mutually
   * exclusive with `w`/`h`/`fit`/`format`/`q` below — `thumb` wins if
   * both are given. */
  thumb?: string;
  download?: boolean;
  token?: string;
  /** Target width/height for an image transform — independent of
   * `thumb`, and the only way to also change `format`/`q`uality. */
  w?: number;
  h?: number;
  fit?: "cover" | "contain" | "inside";
  format?: "jpeg" | "png" | "webp";
  /** 1-100; ignored for lossless formats. */
  q?: number;
}

export interface UploadOptions {
  collection: string;
  field: string;
  /** Set when uploading into an *existing* record's field (an update);
   * omit for a new record (a create) — the returned `recordId` must then
   * be sent as the new record's own `id`, see `UploadResult`'s doc. */
  recordId?: string;
  /** Called with a 0-1 fraction as the PUT to storage progresses.
   * Requires `XMLHttpRequest` (browsers) — a no-op (but still-working)
   * upload in environments without it (Node/SSR), which just resolves
   * once the whole body is sent. */
  onProgress?: (fraction: number) => void;
  signal?: AbortSignal;
}

export interface UploadResult {
  /** Pass this as the file field's value in the record `create`/`update`
   * call that follows — the server resolves it to the real stored
   * filename and verifies the object landed in storage with the right
   * size. Single-use: a second create/update reusing it is refused. */
  token: string;
  /** For a create-shaped upload (no `recordId` given), this is the id
   * the *new* record must be created with (`cb.collection(...).create({
   * id: result.recordId, ... })`) — the token was reserved against it,
   * and a different id won't resolve. */
  recordId: string;
  /** The final stored filename (informational — read it back off the
   * created/updated record after the token resolves, don't assume it in
   * advance). */
  filename: string;
}

interface PresignResponse {
  token: string;
  recordId: string;
  uploadUrl: string;
  uploadMethod: string;
  filename: string;
  expiresAt: string;
}

/** `PUT file` to `url`, reporting progress via `XMLHttpRequest` when it's
 * available (every browser; not Node), falling back to a plain `fetch`
 * PUT (still correct, just no progress callbacks) otherwise. */
function putWithProgress(
  url: string,
  body: Blob,
  onProgress?: (fraction: number) => void,
  signal?: AbortSignal,
): Promise<void> {
  if (typeof XMLHttpRequest === "undefined") {
    return fetch(url, { method: "PUT", body, signal }).then((res) => {
      if (!res.ok) throw new Error(`Upload failed with status ${res.status}.`);
    });
  }
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("PUT", url, true);
    if (onProgress) {
      xhr.upload.onprogress = (e) => {
        if (e.lengthComputable) onProgress(e.loaded / e.total);
      };
    }
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        onProgress?.(1);
        resolve();
      } else {
        reject(new Error(`Upload failed with status ${xhr.status}.`));
      }
    };
    xhr.onerror = () => reject(new Error("Upload failed: network error."));
    xhr.onabort = () => reject(new DOMException("Upload aborted.", "AbortError"));
    signal?.addEventListener("abort", () => xhr.abort());
    xhr.send(body);
  });
}

export class FilesService {
  private readonly transport: Transport;
  private readonly authHeader: () => string | undefined;

  constructor(transport: Transport, authHeader: () => string | undefined) {
    this.transport = transport;
    this.authHeader = authHeader;
  }

  url(record: Pick<RecordModel, "collectionId" | "id">, filename: string, options: FileURLOptions = {}): string {
    const path = `/api/files/${encodeURIComponent(record.collectionId)}/${encodeURIComponent(record.id)}/${encodeURIComponent(filename)}`;
    const query = new URLSearchParams();
    if (options.thumb) {
      query.set("thumb", options.thumb);
    } else {
      if (options.w) query.set("w", String(options.w));
      if (options.h) query.set("h", String(options.h));
      if (options.fit) query.set("fit", options.fit);
      if (options.format) query.set("format", options.format);
      if (options.q) query.set("q", String(options.q));
    }
    if (options.download) query.set("download", "1");
    if (options.token) query.set("token", options.token);
    const qs = query.toString();
    return qs.length > 0 ? `${this.transport.buildURL(path)}?${qs}` : this.transport.buildURL(path);
  }

  async token(): Promise<string> {
    const res = await this.transport.send<{ token: string }>(
      "/api/files/token",
      { method: "POST" },
      this.authHeader,
    );
    return res.token;
  }

  /** Upload `file` straight to storage (S3, signed — or, for the local
   * driver, a same-origin route the server handles itself; the flow
   * below is identical either way) and get back a single-use token: pass
   * it as the target field's value in the `create`/`update` call that
   * follows and the server resolves it to the real stored file. Splits
   * the upload out of the record write itself so a large file's bytes
   * never round-trip through this server's own request handler, and so
   * `onProgress` can report on the PUT alone rather than an opaque
   * multipart POST.
   *
   * ```ts
   * const upload = await cb.files.upload(file, { collection: "posts", field: "cover" });
   * await cb.collection("posts").create({ id: upload.recordId, title: "...", cover: upload.token });
   * ```
   */
  async upload(file: Blob & { name?: string }, options: UploadOptions): Promise<UploadResult> {
    const filename = file.name ?? "upload";
    const presigned = await this.transport.send<PresignResponse>(
      "/api/files/presign",
      {
        method: "POST",
        body: {
          collection: options.collection,
          field: options.field,
          filename,
          contentType: file.type || "application/octet-stream",
          size: file.size,
          recordId: options.recordId,
        },
        signal: options.signal,
      },
      this.authHeader,
    );
    const uploadUrl = /^https?:\/\//.test(presigned.uploadUrl)
      ? presigned.uploadUrl
      : this.transport.buildURL(presigned.uploadUrl);
    await putWithProgress(uploadUrl, file, options.onProgress, options.signal);
    return {
      token: presigned.token,
      recordId: presigned.recordId,
      filename: presigned.filename,
    };
  }
}
