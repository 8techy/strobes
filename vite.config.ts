import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri serves the frontend from a fixed port and needs a stable dev server.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 5174 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    // Tauri ships a fixed WebView2/WebKit version, so there is no need to
    // transpile for older browsers.
    target: "chrome110",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    // Left as a boolean so Vite picks its default minifier; naming esbuild
    // explicitly would require installing it separately.
    minify: !process.env.TAURI_ENV_DEBUG,
  },
});
