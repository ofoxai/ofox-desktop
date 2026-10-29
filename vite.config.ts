import path from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import { codeInspectorPlugin } from "code-inspector-plugin";

/**
 * KaTeX's stylesheet offers every font as woff2, woff and ttf, and Vite would
 * bundle all three. Both webviews read woff2, so the other two are dropped
 * (the app carries about 0.8 MB less). Fails the build if KaTeX changes the
 * stylesheet so that this no longer holds.
 */
function katexWoff2Only(): Plugin {
  return {
    name: "katex-woff2-only",
    enforce: "pre",
    transform(code, id) {
      if (!/katex\/dist\/katex(?:\.min)?\.css$/.test(id.split("?")[0]))
        return null;
      const css = code.replace(
        /,\s*url\([^)]*\.(?:woff|ttf)\)\s*format\("(?:woff|truetype)"\)/g,
        "",
      );
      if (/\.(?:woff|ttf)\)/.test(css) || !css.includes(".woff2"))
        throw new Error(`${id}: could not reduce its fonts to woff2`);
      return { code: css, map: null };
    },
  };
}

export default defineConfig(({ command }) => ({
  root: "src",
  plugins: [
    command === "serve" &&
      codeInspectorPlugin({
        bundler: "vite",
      }),
    react(),
    katexWoff2Only(),
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
