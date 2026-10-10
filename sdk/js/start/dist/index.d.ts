import { type AnySchema, type CratebaseClient, type RecordModel } from "@cratebase/client";
export interface CratebaseStartOptions {
    /** Cratebase base URL, e.g. `http://127.0.0.1:8090`. */
    url: string;
    /** A `cb_…` API key, used by `adminClient()`. */
    apiKey?: string;
    /** Session cookie name. Default `"cb_session"`. */
    cookieName?: string;
    /** Auth collection backing the session. Default `"users"`. */
    authCollection?: string;
    /** Session cookie lifetime. Default 30 days. */
    maxAgeSeconds?: number;
    /** How long `getUser()` trusts a verified token. Default 240 000 ms. */
    sessionCacheTtlMs?: number;
}
export declare function createCratebaseStart<S extends AnySchema = Record<string, RecordModel>>(opts: CratebaseStartOptions): {
    client: () => CratebaseClient<S>;
    adminClient: () => CratebaseClient<S>;
    getUser: (options?: {
        fresh?: boolean;
    }) => Promise<RecordModel | null>;
    setSession: (token: string) => void;
    clearSession: () => void;
    authMiddleware: import("@tanstack/react-start").RequestMiddlewareAfterServer<{}, undefined, {
        user: RecordModel;
    }>;
    freshAuthMiddleware: import("@tanstack/react-start").RequestMiddlewareAfterServer<{}, undefined, {
        user: RecordModel;
    }>;
    optionalAuthMiddleware: import("@tanstack/react-start").RequestMiddlewareAfterServer<{}, undefined, {
        user: RecordModel | null;
    }>;
};
//# sourceMappingURL=index.d.ts.map