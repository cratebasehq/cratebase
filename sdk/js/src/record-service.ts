import type { Cratebase } from "./client.js";
import type { AuthMethodsResponse, AuthResponse, ListOptions, ListResult, RecordModel } from "./types.js";

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

  /** `identity` is whatever the collection's configured identity field is
   * (email by default, but could be a username). */
  async authWithPassword(identity: string, password: string): Promise<AuthResponse<T>> {
    const result = await this.client.send<AuthResponse<T>>(`/api/collections/${this.collectionIdOrName}/auth-with-password`, {
      method: "POST",
      body: { identity, password },
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

  /** Lists available auth methods. Pass your app's OAuth2 redirect
   * URI/deep link and each provider's `authUrl` comes back ready to
   * `open()` directly. */
  async listAuthMethods(redirectUri?: string): Promise<AuthMethodsResponse> {
    return this.client.send<AuthMethodsResponse>(`/api/collections/${this.collectionIdOrName}/auth-methods`, {
      method: "GET",
      query: { redirectUri },
    });
  }

  /** Completes an OAuth2 login: exchange the `code` your app received at
   * `redirectUri` (the same one used to build the `authUrl` from
   * `listAuthMethods`) for a session. */
  async authWithOAuth2(provider: string, code: string, redirectUri: string): Promise<AuthResponse<T>> {
    const result = await this.client.send<AuthResponse<T>>(`/api/collections/${this.collectionIdOrName}/auth-with-oauth2`, {
      method: "POST",
      body: { provider, code, redirectUri },
    });
    this.client.authStore.save(result.token, result.record);
    return result;
  }

  /** Sends a verification email if `email` matches an account — always
   * resolves regardless, so it can't be used to enumerate accounts. */
  async requestVerification(email: string): Promise<void> {
    await this.client.send<void>(`/api/collections/${this.collectionIdOrName}/request-verification`, {
      method: "POST",
      body: { email },
    });
  }

  /** Confirms a verification token from the emailed link. */
  async confirmVerification(token: string): Promise<void> {
    await this.client.send<void>(`/api/collections/${this.collectionIdOrName}/confirm-verification`, {
      method: "POST",
      body: { token },
    });
  }

  /** Sends a password reset email if `email` matches an account — always
   * resolves regardless, so it can't be used to enumerate accounts. */
  async requestPasswordReset(email: string): Promise<void> {
    await this.client.send<void>(`/api/collections/${this.collectionIdOrName}/request-password-reset`, {
      method: "POST",
      body: { email },
    });
  }

  /** Confirms a password reset token and sets a new password. */
  async confirmPasswordReset(token: string, password: string, passwordConfirm: string): Promise<void> {
    await this.client.send<void>(`/api/collections/${this.collectionIdOrName}/confirm-password-reset`, {
      method: "POST",
      body: { token, password, passwordConfirm },
    });
  }

  /** Requires the current session's record to be authenticated. Sends a
   * confirmation link to `newEmail` — the identity only changes once that
   * link is confirmed, proving ownership of the new address. */
  async requestEmailChange(newEmail: string): Promise<void> {
    await this.client.send<void>(`/api/collections/${this.collectionIdOrName}/request-email-change`, {
      method: "POST",
      body: { newEmail },
    });
  }

  /** Confirms an email-change token from the emailed link. */
  async confirmEmailChange(token: string): Promise<void> {
    await this.client.send<void>(`/api/collections/${this.collectionIdOrName}/confirm-email-change`, {
      method: "POST",
      body: { token },
    });
  }
}
