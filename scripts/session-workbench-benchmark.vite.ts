import path from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const scripts = path.dirname(fileURLToPath(import.meta.url));
const native = path.join(scripts, "session-workbench-benchmark-native.ts");
export default defineConfig({
  root: scripts,
  publicDir: false,
  plugins: [react()],
  base: "./",
  resolve: {
    alias: [
      { find: "@", replacement: path.join(scripts, "../src") },
      ...[
        "@tauri-apps/api/core",
        "@tauri-apps/api/event",
        "@tauri-apps/plugin-dialog",
      ].map((find) => ({ find, replacement: native })),
    ],
  },
  css: { postcss: path.join(scripts, "..") },
  build: {
    outDir: path.join(scripts, "../dist/session-workbench-benchmark"),
    emptyOutDir: true,
    rollupOptions: {
      input: path.join(scripts, "session-workbench-benchmark.html"),
    },
  },
  preview: { host: "127.0.0.1", port: 4174, strictPort: true },
});
