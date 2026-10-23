import { resolve } from "node:path"
import { paraglideVitePlugin } from "@inlang/paraglide-js"
import tailwindcss from "@tailwindcss/vite"
import { tanstackRouter } from "@tanstack/router-plugin/vite"
import react from "@vitejs/plugin-react"
import { defineConfig, loadEnv, type UserConfig } from "vite"

import { configuredApiOrigin } from "./src/lib/api-origin.ts"

// The API is same-origin, or on the origin a build names in
// `VITE_MESHWAY_API_ORIGIN` (`src/lib/api-origin.ts`). In development the
// browser always talks to `/api` on its own origin: the development server
// proxies it to the Server named here; there is no default, so it never
// reaches another Server by accident.
const controller = process.env.MESHWAY_CONTROLLER_URL

const config = {
  plugins: [
    tanstackRouter({ target: "react", autoCodeSplitting: true }),
    paraglideVitePlugin({
      project: "./project.inlang",
      outdir: "./src/paraglide",
      // Keep in sync with the `i18n` script in package.json.
      strategy: ["localStorage", "preferredLanguage", "baseLocale"],
      emitTsDeclarations: true,
      outputStructure: "message-modules",
    }),
    react(),
    tailwindcss(),
  ],
  resolve: {
    alias: {
      "@": resolve(import.meta.dirname, "./src"),
    },
  },
  server: {
    proxy: controller
      ? {
          "/api": {
            target: controller,
            changeOrigin: false,
            // Node sends a response's headers with its first body bytes; a
            // resumed event stream may send none for a while, and the
            // console would not know the controller took its cursor. Its
            // headers go out at once, as the controller sends them.
            configure: (proxy) =>
              proxy.on("proxyRes", (upstream, _request, response) => {
                if (
                  upstream.headers["content-type"]?.startsWith(
                    "text/event-stream"
                  )
                )
                  queueMicrotask(() => response.flushHeaders())
              }),
          },
        }
      : undefined,
  },
} satisfies UserConfig

export default defineConfig(({ command, mode, isPreview }) => {
  // `vite preview` serves a build, which carries its own API origin.
  if (command === "serve" && !isPreview && !controller)
    throw new Error(
      "Set MESHWAY_CONTROLLER_URL to the V2 controller's origin (http://127.0.0.1:<port>)."
    )
  // A malformed API origin fails the build rather than the browser.
  if (command === "build")
    configuredApiOrigin(
      loadEnv(mode, import.meta.dirname, "VITE_").VITE_MESHWAY_API_ORIGIN
    )
  return config
})
