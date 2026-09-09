import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// This example runs its own Vite dev server (`npm run dev`) rather than
// being served by the shared static server in `examples/serve.sh` — see
// README.md for why. Port 5174 is deliberately distinct from
// `web/admin`'s dev server (5173) so both can run at once.
export default defineConfig({
  plugins: [react()],
  resolve: {
    // `@cratebase/client` isn't published to npm yet — this example lives
    // in the Cratebase monorepo, so it resolves the SDK straight from
    // source, the same alias `web/admin/vite.config.ts` uses. Cloning
    // this directory standalone (outside the monorepo) needs the real
    // published package instead once it ships.
    alias: {
      "@cratebase/client": fileURLToPath(new URL("../../sdk/js/client/src/index.ts", import.meta.url)),
    },
  },
  server: {
    port: 5174,
  },
});
