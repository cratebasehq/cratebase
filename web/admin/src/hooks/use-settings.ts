import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { cb } from "@/lib/api";

/**
 * `GET /api/settings`, as the server actually returns it.
 *
 * Secrets are deliberately absent from the response — `smtp.password` and
 * `s3.secret` are never sent back — so every form over this has to treat
 * them as write-only: blank means "leave whatever is stored alone".
 */
export interface ServerSettings {
  meta: {
    appName: string;
    appURL: string;
    senderName: string;
    senderAddress: string;
    hideControls: boolean;
    accentColor: string;
  };
  logs: { maxDays: number; minLevel: number; logIP: boolean; logAuthId: boolean; maxDataSize: number };
  batch: { enabled: boolean; maxRequests: number; timeout: number; maxBodySize: number };
  smtp: {
    enabled: boolean;
    host: string;
    port: number;
    username: string;
    authMethod: string;
    tls: boolean;
    localName: string;
    password?: string;
  };
  s3: {
    enabled: boolean;
    bucket: string;
    region: string;
    endpoint: string;
    accessKey: string;
    forcePathStyle: boolean;
    secret?: string;
  };
  backups: {
    cron: string;
    cronMaxKeep: number;
    s3: ServerSettings["s3"];
  };
  llm: {
    enabled: boolean;
    provider: string;
    baseUrl: string;
    model: string;
    apiKey?: string;
  };
  rateLimits: {
    enabled: boolean;
    excludedIPs: string[];
    rules: { label: string; audience: string; duration: number; maxRequests: number }[];
  };
  trustedProxy: { headers: string[]; useLeftmostIP: boolean };
  superuserIPs: string[];
  /** Toggle-gated built-in module (`crates/server/src/teams.rs`): off by
   * default, no dashboard page of its own yet — the `_teams`/
   * `_team_members` system collections stay hidden from the sidebar's
   * System group until this is `true`. */
  teams: { enabled: boolean };
  /** Toggle-gated built-in Queue plugin (`crates/server/src/queue.rs`):
   * off by default, no dashboard page of its own yet. Enable with
   * `PATCH /api/settings { "queue": { "enabled": true } }`. */
  queue: { enabled: boolean };
  push: {
    vapid: { enabled: boolean; publicKey: string; subject: string; privateKey?: string };
    fcm: { enabled: boolean; serviceAccountJson?: string };
    apns: {
      enabled: boolean;
      keyId: string;
      teamId: string;
      bundleId: string;
      production: boolean;
      key?: string;
    };
    triggers: {
      enabled: boolean;
      collection: string;
      events: string;
      title: string;
      body: string;
      targetField: string;
    }[];
  };
}

/** Settings-gated nav visibility, additive to `lib/settings-nav.ts`'s
 * static registry: a toggle-gated built-in module's page (currently just
 * `/settings/llm`) is hidden from the sidebar and command palette until
 * its flag is on, even though the route itself still exists. */
export function isSettingsItemVisible(to: string, settings: ServerSettings | undefined): boolean {
  if (to === "/settings/llm") return Boolean(settings?.llm.enabled);
  return true;
}

export const SETTINGS_KEY = ["settings"] as const;

export function useSettings() {
  return useQuery({
    queryKey: SETTINGS_KEY,
    queryFn: async () => (await cb.settings.getAll()) as unknown as ServerSettings,
    staleTime: 30_000,
  });
}

/** A partial settings update. The server merges top-level keys, so a form
 * only sends the section it owns. */
export function useSettingsMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (patch: Record<string, unknown>) =>
      (await cb.settings.update(patch)) as unknown as ServerSettings,
    onSuccess: (settings) => {
      queryClient.setQueryData(SETTINGS_KEY, settings);
      // The grid's bulk delete reads `batch` through its own key.
      void queryClient.invalidateQueries({ queryKey: ["settings", "batch"] });
    },
  });
}
