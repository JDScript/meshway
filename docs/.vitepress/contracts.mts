import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";

const source = fileURLToPath(new URL("../../packages/protocol/openapi.json", import.meta.url));
const asset = "v2/contracts/openapi.json";

// Publish the existing generated contract verbatim; no second schema source.
export function contractAssets(): Plugin {
  let ssr = false;
  return {
    name: "meshway-contract-assets",
    configResolved(config) { ssr = Boolean(config.build.ssr); },
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        if (request.url?.split("?")[0] !== `/${asset}` || !["GET", "HEAD"].includes(request.method ?? "")) return next();
        void readFile(source).then(bytes => {
          response.setHeader("Content-Type", "application/json; charset=utf-8");
          response.setHeader("Cache-Control", "no-cache");
          response.end(request.method === "HEAD" ? undefined : bytes);
        }).catch(next);
      });
    },
    async generateBundle() {
      if (!ssr) this.emitFile({ type: "asset", fileName: asset, source: await readFile(source) });
    },
  };
}
