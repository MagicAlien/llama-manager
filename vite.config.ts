import { fileURLToPath, URL } from "node:url";

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://tauri.app/start/frontend/vite/
export default defineConfig(async () => ({
  plugins: [react()],

  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },

  // Vite options tailored for Tauri development.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Tell Vite to ignore watching `src-tauri`.
      ignored: ["**/src-tauri/**"],
    },
  },

  test: {
    environment: "jsdom",
    passWithNoTests: true,
  },
}));
