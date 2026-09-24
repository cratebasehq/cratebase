import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Neither `@cratebase/client` nor `@cratebase/react` is published to npm
// yet — this example lives in the Cratebase monorepo, so (like
// examples/kanban) it resolves both straight from source. Cloning this
// directory standalone needs the real published packages once they ship.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@cratebase/client": fileURLToPath(new URL("../../sdk/js/client/src/index.ts", import.meta.url)),
      "@cratebase/react": fileURLToPath(new URL("../../sdk/js/react/src/index.ts", import.meta.url)),
    },
  },
  server: {
    // Distinct from web/admin (5173) and examples/kanban (5174).
    port: 5175,
  },
});
