import { resolve } from "node:path"
import { defineConfig } from "vitest/config"

// Domain tests: plain modules in Node, no browser. The `test` script
// compiles the messages first, as `typecheck` and `build` do.
export default defineConfig({
  resolve: {
    alias: {
      "@": resolve(import.meta.dirname, "./src"),
    },
  },
  test: {
    include: ["tests/**/*.test.ts"],
    environment: "node",
  },
})
