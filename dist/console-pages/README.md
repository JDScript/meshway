# Console on Cloudflare Pages

The Web Console is a static SPA. Hosted on Cloudflare Pages it lives on its own
origin and calls the Server cross-origin; the other hosting form (a reverse
proxy serving the Console and the Server on one origin) is in
[`docs/v2/install.md`](../../docs/v2/install.md#server-proxy).

| Piece | Value |
| --- | --- |
| Pages project | `meshway-console` (direct upload, production branch `main`) |
| Console origin | `https://meshway.jdscript.dev` (custom domain of the project) |
| Server (API) origin | `https://meshway-prod-api.jdscript.dev` |

## Server

The Server must list the Console's origin, otherwise the browser refuses every
response:

```sh
# /etc/meshway-server/server.env
MESHWAY_CONSOLE_ORIGIN=https://meshway.jdscript.dev
```

This also makes the device approval links point at the Console. Cross-origin
rules (only `/api/v1/`, bearer token, no cookies) are in
[`docs/v2/server.md`](../../docs/v2/server.md#cors). A `*.pages.dev` preview is
not that origin, so a preview deployment builds and loads but cannot sign in
against the production Server.

## Deploy

```sh
mise run deploy:console-pages                     # production (main)
mise run deploy:console-pages -- --branch preview # preview: preview.meshway-console.pages.dev
mise run deploy:console-pages -- --build-only
```

`deploy.sh` builds with `VITE_MESHWAY_API_ORIGIN` (default
`https://meshway-prod-api.jdscript.dev`; override with `MESHWAY_API_ORIGIN`)
and `VITE_MESHWAY_SERVER_URL` (the Node install commands; default the same
origin), checks the bundle contains the API origin and `_redirects`, creates
the Pages project when it does not exist, then runs `wrangler pages deploy`.
Wrangler runs pinned through `pnpm dlx` and uses the logged-in account
(`wrangler login`); set `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` for
a non-interactive run. Set `VITE_MESHWAY_RELEASE` to pin the release tag in
the install commands.

`public/_redirects` (`/* /index.html 200`) gives every client route the SPA;
existing files (assets, `boot.js`) are served as they are.

## Custom domain

Wrangler cannot attach a custom domain to a Pages project. Once, in the
Cloudflare dashboard:

1. Workers & Pages → `meshway-console` → Custom domains → Set up a custom domain.
2. Enter `meshway.jdscript.dev` → Continue → Activate domain. The
   `jdscript.dev` zone is on the same account, so Cloudflare creates the
   proxied `CNAME meshway → meshway-console.pages.dev` record and the
   certificate itself.
3. Wait until the domain shows Active.

The same through the API, with a token that has *Cloudflare Pages: Edit*:

```sh
curl -fsS -X POST \
  "https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID/pages/projects/meshway-console/domains" \
  -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" -H 'Content-Type: application/json' \
  --data '{"name":"meshway.jdscript.dev"}'
```

## Verify

```sh
# CORS for the Console origin (the Server must run a build with Console-origin CORS)
curl -sS -o /dev/null -D - -X OPTIONS https://meshway-prod-api.jdscript.dev/api/v1/auth/login \
  -H 'Origin: https://meshway.jdscript.dev' \
  -H 'Access-Control-Request-Method: POST' \
  -H 'Access-Control-Request-Headers: content-type' | grep -i '^access-control-'
# SPA fallback on a client route
curl -sS https://meshway.jdscript.dev/networks | grep -c '<div id="root">'
```

Then sign in at `https://meshway.jdscript.dev`, open a network, and check in
the browser's network panel that `…/api/v1/networks/<id>/events` stays open
(`text/event-stream`) and that live changes arrive.
