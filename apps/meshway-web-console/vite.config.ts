import { resolve } from "node:path"
import { paraglideVitePlugin } from "@inlang/paraglide-js"
import tailwindcss from "@tailwindcss/vite"
import { tanstackRouter } from "@tanstack/router-plugin/vite"
import react from "@vitejs/plugin-react"
import { defineConfig, type UserConfig } from "vite"

// Same origin in development as in production, where the controller serves
// this build: the browser only ever talks to `/api` on its own origin. The
// development server proxies it to the V2 controller named here; there is no
// default, so it never reaches another controller by accident.
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

export default defineConfig(({ command }) => {
  if (command === "serve" && !controller)
    throw new Error(
      "Set MESHWAY_CONTROLLER_URL to the V2 controller's origin (http://127.0.0.1:<port>)."
    )
  return config
})
