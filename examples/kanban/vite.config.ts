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
    // Neither `@cratebase/client` nor `@cratebase/react` is published to
    // npm yet — this example lives in the Cratebase monorepo, so it
    // resolves both straight from source, the same alias
    // `web/admin/vite.config.ts` uses for the client. Cloning this
    // directory standalone (outside the monorepo) needs the real
    // published packages instead once they ship.
    alias: {
      "@cratebase/client": fileURLToPath(new URL("../../sdk/js/client/src/index.ts", import.meta.url)),
      "@cratebase/react": fileURLToPath(new URL("../../sdk/js/react/src/index.ts", import.meta.url)),
    },
  },
  server: {
    port: 5174,
  },
});
