import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri dev server: listen on all interfaces so the Tauri webview can reach
// it from inside the Docker container.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: "0.0.0.0",
    hmr: { port: 5174 },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
    outDir: "dist",
    emptyOutDir: true,
  },
});
