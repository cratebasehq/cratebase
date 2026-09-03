/** Shape of every non-2xx JSON response from the Cratebase API. */
export interface ApiErrorBody {
  code: number;
  message: string;
  data: Record<string, string>;
}

/** Thrown for any non-2xx response. `data` carries per-field validation
 * errors when `status === 400`. */
export class ClientResponseError extends Error {
  readonly status: number;
  readonly data: Record<string, string>;
  readonly url: string;

  constructor(url: string, status: number, body: Partial<ApiErrorBody> | undefined) {
    super(body?.message || `request to ${url} failed with status ${status}`);
    this.name = "ClientResponseError";
    this.url = url;
    this.status = status;
    this.data = body?.data ?? {};
  }
}
