# API reference generation

`pnpm --filter @meshway/web-site generate:api` reads the generated contract at
`packages/protocol/openapi.json`. The site's `dev`, `build` and `typecheck`
scripts run it first.

- `src/lib/openapi.ts` owns the filter: `/api/v1` only, excluding the
  `execution`, `node` and `connectivity` tags. The renderer uses the same view.
- `generate-api.mjs` writes thin MDX pages and sidebar metadata under
  `content/docs/api/<tag>/`. These directories and `api/meta*.json` are ignored
  and regenerated. The two authored `api/index*.mdx` overview pages are kept.
- Each operation gets Chinese and English wrappers with identical stable
  `<tag>/<kebab-case-operationId>` slugs. Both use the same schema renderer and
  code generator. Add a Chinese title when the contract adds an operation.
- `src/lib/openapi-samples.ts` generates curl and Node.js TypeScript calls.
  Bodies come from a reader-supplied `request.json`, validated with the SDK's
  exported Zod schema. No synthetic request or response values are presented
  as contract examples. Generation rejects unsupported argument shapes and
  operations missing from the SDK.
- Page payloads retain only reachable schemas. Annotated `$ref` occurrences
  get isolated presentation components because Fumadocs v12's shared Scalar
  proxies can otherwise overwrite descriptions at other occurrences. The
  original contract is never changed. Union labels use their declared
  `type`, `kind` or `code` enum value.
- `src/lib/openapi-options.ts` is the single playground switch. It remains
  disabled until delegated sign-in and the server CORS contract exist.

Contract changes during a running dev session require rerunning generation
and restarting the server. `mise run check:site` verifies types, lint,
formatting and the complete static export.
