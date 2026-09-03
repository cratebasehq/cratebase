import type { Cratebase } from "./client.js";
import type { AdminAuthResponse } from "./types.js";

/** Superuser (admin panel) session management. */
export class AdminService {
  constructor(private readonly client: Cratebase) {}

  async authWithPassword(email: string, password: string): Promise<AdminAuthResponse> {
    const result = await this.client.send<AdminAuthResponse>("/api/admins/auth-with-password", {
      method: "POST",
      body: { email, password },
    });
    this.client.authStore.save(result.token, result.admin);
    return result;
  }

  async authRefresh(): Promise<AdminAuthResponse> {
    const result = await this.client.send<AdminAuthResponse>("/api/admins/auth-refresh", { method: "POST" });
    this.client.authStore.save(result.token, result.admin);
    return result;
  }
}
