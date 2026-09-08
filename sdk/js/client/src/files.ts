/** `/api/files/*`: building a file's URL and minting protected-file
 * tokens. */

import type { Transport } from "./transport.js";
import type { RecordModel } from "./types.js";

export interface FileURLOptions {
  /** `WxH`, `WxHt|b|f` (crop to top/bottom/fit), `0xH`, or `Wx0` — the
   * thumb grammar `crates/server/src/routes/files.rs` accepts. */
  thumb?: string;
  download?: boolean;
  token?: string;
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
    if (options.thumb) query.set("thumb", options.thumb);
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
}
