import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { codeInspectorPlugin } from "code-inspector-plugin";

export default defineConfig(({ command }) => ({
  root: "src",
  plugins: [
    command === "serve" &&
      codeInspectorPlugin({
        bundler: "vite",
      }),
    react(),
  ].filter(Boolean),
  base: "./",
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    // The CSP has no `data:` for fonts, so a small font (KaTeX_Size3) inlined
    // into the CSS would be blocked; fonts always stay files from the app.
    assetsInlineLimit: (file) =>
      /\.(?:woff2?|ttf|otf)$/i.test(file) ? false : undefined,
  },
  server: {
    port: 3011,
    strictPort: true,
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_"],
}));

