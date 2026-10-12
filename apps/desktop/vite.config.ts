import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri loads the dev server from a fixed port (not the usual 1420, which other Tauri apps
// take) and shows its own errors, so Vite keeps both.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1430, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  // WebView2 on Windows, WebKitGTK on Linux and WKWebView on macOS all run ES2022.
  build: { target: "es2022" },
});
