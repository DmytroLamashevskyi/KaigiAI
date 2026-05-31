import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Frontend dev server. Later wrapped by Tauri; keep a fixed port for the webview.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
});
