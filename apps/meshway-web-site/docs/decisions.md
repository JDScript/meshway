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
said once on the overview page, not on each field.

## Trying operations

Each operation page can send its request as the reader, with a session
the console delegates (`lib/try-it.ts`): "authorize in the console" goes
to the console's `/authorize-site` with a PKCE challenge (S256) and a
state; on consent the console returns to `/auth/callback` (`/en/auth/callback`)
with a one-time code, which is exchanged once (`exchangeDelegation`) for a
one-hour session, and the page that started is opened again. The verifier
and state wait in `sessionStorage` for that round trip only; the token is
held in memory only, so a reload or a change of language (a full page
load) drops it and authorizing again is one click while the console is
signed in. The server knows the site as the client `docs`
(`--delegation-client docs=<origin>`, which also allows its origin
cross-origin). Requests go to the console's origin, where the server's API
is served; the session never becomes a field of Fumadocs' playground (it
keeps those in localStorage) but is added to each request, a write is
confirmed before it goes, a 401 drops the session, and "stop trying" ends
it at the server. The playground's response examples sampled from the
schema are hidden: only a real response shows.

A JSON response opens as a tree (`components/try/json-tree.tsx`):
objects and arrays fold (two levels open, fifty items at a time), values
are coloured by type, and each field's description from the contract
follows its value, muted and cut to the line (in full on hover); one
switch hides them, and the raw view is Fumadocs' own. The schema is the
operation's for the status received (none for a status it does not
declare); the value picks the way through it (`lib/schema-walk.ts`): a
union's branch by its `type`, `kind` or `code`, else by its required
fields, a nullable's non-null side. An event stream (`watchNetwork`) is
tried in its own panel, since the playground waits for a whole body: a
network of the reader's (read with the session), an optional cursor to
resume from, connect and stop; the generated SDK checks each event
against the contract, the latest 200 are kept newest first, and each
opens as the same tree with the event schema. `lib/openapi-options.ts`
turns trying off.
