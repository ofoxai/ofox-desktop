import path from "node:path";
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./tests/setupGlobals.ts", "./tests/setupTests.ts"],
    globals: true,
    // Integration tests render the complete settings/app trees. A five-second
    // per-test default is too tight on shared CI runners even when each async
    // assertion has its own bounded wait.
    testTimeout: 15_000,
    coverage: {
      reporter: ["text", "lcov"],
    },
  },
});
