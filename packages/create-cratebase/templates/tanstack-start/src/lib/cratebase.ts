// The app's Cratebase helper. `createCratebaseStart` gives every server
// function a request-scoped client that reads the visitor's session cookie,
// plus the auth middlewares. Tokens live in an HttpOnly cookie, never in
// browser JS, and every call below runs on the server.
//
// Why this file is not named `*.server.ts`: server functions reference
// `cb.authMiddleware` at module scope, so route modules (which the browser
// bundle also loads) import this file. The code that actually uses the
// request (`cb.client()`, `cb.getUser()`) only runs inside server handlers.
import { createCratebaseStart } from "@cratebase/start";
import type { Schema } from "../../cratebase-types";

// On Workers, `.env` (dev) and secrets (prod) appear in `process.env` thanks to
// the `nodejs_compat` flag in wrangler.jsonc. The guard keeps this module
// loadable in the browser bundle, where `process` doesn't exist.
const vars: Record<string, string | undefined> = typeof process === "undefined" ? {} : process.env;

export const cb = createCratebaseStart<Schema>({
  url: vars.CRATEBASE_URL ?? "http://127.0.0.1:8090",
  apiKey: vars.CRATEBASE_API_KEY || undefined,
});
