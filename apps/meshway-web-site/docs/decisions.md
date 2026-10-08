# Site decisions

The public site: the landing page, the user docs and the API reference.
Static (`output: "export"`), Next.js with Fumadocs. `docs/v2/` stays the
internal developer specification; nothing here is copied from it as a
second definition, and nothing here says more than the product does.

## Structure

- `apps/meshway-web-site` (`@meshway/web-site`) holds the site; the console
  is only the application.
- `packages/meshway-ui` (`@meshway/ui`) holds what the console and the site
  share: the theme (palette light and dark, type roles, radii, easing, the
  glass, base element styles, Inter), `cn` that knows the theme's scales,
  `Button` / `buttonVariants` and `BrandMark`. Each app keeps its own
  tokens and components; something moves into the package when both use
  it, never as a copy.
- Fumadocs' colours come from the shared palette (`fumadocs-ui/css/shadcn.css`
  maps them to the shadcn variables the theme defines).

## Languages

Chinese and English, Chinese first: `/` and `/docs/…` are Chinese, `/en`
and `/en/docs/…` English. Each language is its own root layout (`(zh)/`,
`en/`), so the static export needs no middleware. A page `x.mdx` is
Chinese and `x.en.mdx` its English; Fumadocs' own words are translated in
`lib/translations.ts`. Search is static (Flexsearch), Chinese with the CJK
encoder; the site queries it with its own client (`lib/search-client.ts`),
which encodes queries the way each language's index was built (Fumadocs'
static client uses the default encoder, so Chinese found nothing). The API reference's field descriptions come from the contract
and are English in both.

## Navigation

The docs have two tabs: the user guides (`content/docs/(guide)/`, a folder
group, so their URLs stay `/docs/…`) and the API reference
(`content/docs/api/`). The landing's header links the docs, the API
reference and the console; inside the docs the tabs say which part is open
and the sidebar links only the console.

## Links out

The console the site signs in to is `NEXT_PUBLIC_MESHWAY_CONSOLE_URL`, set
per deployment at build time; there is no default (`mise run dev:site` and
the check build use the local console, `http://127.0.0.1:3001`).

## Landing

The product, the docs and signing in to the console. It states what the
product does today only; what is not supported yet (exits, NAT, SOCKS5,
deployment and pricing) is not mentioned.

## API reference

Generated from `packages/protocol/openapi.json` (the published contract),
`/api/v1` only: `/agent-api/v1` needs Node credentials and is not for
people. One page per operation at `/docs/api/<tag>/<operationId in
kebab-case>` (and `/en/…`), generated before `dev`, `build` and
`typecheck` (`scripts/README.md`), with curl and `@meshway/control-client`
samples generated from the operation; the user docs link to these pages
for exact fields instead of restating them. Constraints every ID and
integer carries (the UUID pattern and length, the JSON-safe maximum) are
said once on the overview page, not on each field. Trying an operation will take a delegated session from the console
(authorization code with PKCE: the console asks for consent, the server
issues a one-hour session that cannot change the password or email, revoke
sessions or delegate again); writes are confirmed first. That needs the
server's delegation, token exchange and a CORS allow-list for the site
(console `docs/v2-api-needs.md`); until then the reference has no try-it.
