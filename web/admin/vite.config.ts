import { fileURLToPath, URL } from "node:url";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

// https://vite.dev/config/
export default defineConfig({
  // The server mounts the dashboard at `/_/` (PocketBase's path), so the
  // bundle's asset URLs have to be absolute under that prefix. The Vite
  // default of `/` emits `/assets/...`, which the server does not route —
  // the page then loads, renders nothing, and looks like a blank screen.
  // A relative `./` base is not an option either: the SPA has nested
  // routes, so `/_/collections/posts` would resolve `./assets/...`
  // against `/_/collections/`.
  base: "/_/",
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:8090",
    },
  },
});
