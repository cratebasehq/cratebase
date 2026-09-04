import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// This example runs its own Vite dev server (`npm run dev`) rather than
// being served by the shared static server in `examples/serve.sh` — see
// README.md for why. Port 5174 is deliberately distinct from
// `web/admin`'s dev server (5173) so both can run at once.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5174,
  },
});
