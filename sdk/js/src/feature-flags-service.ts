import type { Cratebase } from "./client.js";

interface CheckFlagResponse {
  key: string;
  enabled: boolean;
}

/** Client for the `feature-flags` plugin (`crates/server/src/plugins/feature_flags.rs`).
 * An unknown key resolves to `false` rather than throwing. */
export class FeatureFlagsService {
  constructor(private readonly client: Cratebase) {}

  async isEnabled(key: string): Promise<boolean> {
    const result = await this.client.send<CheckFlagResponse>(
      `/api/plugins/feature-flags/${encodeURIComponent(key)}`,
    );
    return result.enabled;
  }
}
