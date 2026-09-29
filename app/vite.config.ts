import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// Set by `tauri dev` when targeting a remote device.
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/ — options tailored for Tauri (https://v2.tauri.app/start/frontend/vite/)
export default defineConfig({
  plugins: [svelte()],
  // Keep Rust compiler errors visible.
  clearScreen: false,
  server: {
    // Tauri expects a fixed port (see `devUrl` in src-tauri/tauri.conf.json).
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    // Tauri's webviews: WebView2 (Chromium) on Windows, WebKit on macOS/Linux.
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome111" : "safari16",
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
