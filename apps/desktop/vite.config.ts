import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    // Loopback only; the devcontainer sets BETTERTRICKS_DEV_HOST=0.0.0.0 to forward the port.
    host: process.env.BETTERTRICKS_DEV_HOST ?? "127.0.0.1",
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**", "**/catalog/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    // Tauri 2's supported Linux WebKitGTK baseline handles ES2020. Targeting
    // Safari 13 here asks esbuild to lower destructuring that WebKitGTK already
    // supports and is no longer a useful proxy for the Linux runtime.
    target: "es2020",
    minify: process.env.TAURI_ENV_DEBUG ? false : "oxc",
    sourcemap: Boolean(process.env.TAURI_ENV_DEBUG),
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: "./src/test/setup.ts",
    // Cold module transforms of the full app can exceed vitest's 5 s default per test.
    testTimeout: 30_000,
  },
});
