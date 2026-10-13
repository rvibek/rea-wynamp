import { defineConfig } from "vite";

// Vite config for the Wynamp frontend.
// clearScreen:false keeps cargo/tauri logs visible during dev.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    target: "es2021",
    outDir: "dist",
    emptyOutDir: true,
  },
});
