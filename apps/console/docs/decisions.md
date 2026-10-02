# Console decisions

The Vite console replaces the Next.js console in `apps/web` once it covers the
daily workflows. Decisions are recorded as they are confirmed with the user.

## Foundation

| Topic | Decision |
| --- | --- |
| Scope | Product, flows, visuals and code are rebuilt together. The map direction is kept. |
| Stack | Vite, TanStack Router (file routes, typed params and search), TanStack Query, shadcn (Base UI, preset `b0` = `base-nova`, pointer, full set), Paraglide (zh, en). |
| BFF | None. The browser calls the controller through the generated `@meshway/control-client` with Zod validation. |
| Origin | One origin. The controller serves the built `dist/` with an `index.html` fallback; development proxies `/api` to the controller. App paths (`/authorize-device`, `/api/v1/*`) are unchanged. |
| Copy | Every string lives in `messages/{zh,en}.json`; zh is the base locale. Interface text states facts only: no explanations of what the display means or of how it was drawn. |
| Routes | Resources are nested and plural: `/networks/<network_id>/<page>`, e.g. `/networks/<id>/overview?node=<node_id>`. `/networks/<id>` redirects to its overview; `/` to the remembered or first network. Selection and filters live in validated search params. |

## Sessions

- The controller's opaque `mws_` user session is the browser credential. No JWT:
  logout, password changes, session revocation, member removal and role changes
  must take effect on the next request, so every request is checked server-side
  anyway.
- The session is kept in `localStorage` and sent as `Authorization: Bearer`.
  With no ambient credential there is no CSRF surface and no rendered-sign-in
  cookie fence. This deliberately replaces the earlier "no `localStorage`" rule
  in `docs/local-accounts.md`.
- Each tab copies the session into memory when it loads and never re-reads
  storage per request. A `storage` event that changes the session locks and
  reloads that tab; drafts are keyed by session.
- The instance administrator token is never persisted: memory only.
- Defence against XSS is prevention: strict CSP (`script-src 'self'`, no inline
  or third-party scripts), no raw HTML rendering, no `javascript:` URLs. Node
  names, versions and reasons come from agents and devices and are untrusted.

## Overview globe motion

Motion only shows fresh evidence; nothing animates by default.

| Element | Animated when | Otherwise |
| --- | --- | --- |
| Link flow | Both ends report the channel authenticated within its freshness window. Particles run both ways at a fixed speed: dial direction is not business direction. | One-sided: static amber. Unauthenticated: static red dashed. Configured with no report: static grey dotted. |
| Node breathing | The agent's runtime report is current (within its TTL). | Recent control-plane contact only: solid, no breathing. Neither: dimmed. |
| Rotation | Idle globe mode, nothing selected. | Stops on interaction or selection. |

Speed and density are not tied to RTT or traffic. Traffic is not measured;
when it is, flow may map to it. `prefers-reduced-motion` turns off rotation,
flow and breathing; states stay distinguishable by stroke and text.

## Interaction rules

- No layout shift, ever. Feedback that is not part of a component's own shape
  (sign-in failures, read failures, confirmations) goes to a toast; panels
  reserve every slot and only change values.
- Forms are real forms: named fields, `autocomplete`, Enter submits.
- Floating surfaces use one frosted-glass style (`.glass`): mostly opaque fill,
  strong blur, lit top edge, gradient hairline. Readability first.
- Wrappers that animate in fill backwards only; a lingering filter or opacity
  would turn them into backdrop roots and stop the glass from blurring.

## Globe

- One globe for the whole console, rendered by globe.gl (camera, controls,
  atmosphere, DOM markers) plus our own shader layers (surface, land dots,
  orbit, links, comet pulses). Visual weights come from a `GlobeLook`; colours
  only from theme tokens.
- Unplaced nodes sit on an orbit; with GeoIP they move to the ground.
- The engine chunk is lazy-loaded (~550 kB gzip, mostly three.js and
  three-globe); trimming it is open work.

## Design lab

`/lab?v=<concept>` (development builds only) shows whole-direction concepts
over sample data: Observatory, Cartograph, Aurora and Schematic. They exist to
choose a direction and are removed once one is chosen.

## Modes

- **Globe mode** (`/networks/<id>/overview`): the globe fills the screen. Top
  left, the network chip (switcher, node and link counts, link-state bar);
  top right, console entry and preferences; the selected node's card on the
  right.
- **Console mode** (`/networks/<id>/<page>`): shadcn `sidebar-07`. Added with
  `shadcn@4.21.1 add sidebar-07` (style `base-nova`); it generated
  `app-sidebar`, `nav-main`, `nav-projects`, `nav-user` and `team-switcher`
  and skipped 12 existing primitives. Those five were adapted into
  `features/console/` without their sample data: the team switcher became the
  network switcher, `nav-main` is flat (only pages that exist), `nav-user`
  carries the account, its role here, theme, language and sign-out;
  `nav-projects` is not used. `ui/sidebar.tsx` is unchanged.
- The globe docks into the orb of the network switcher (or of the top bar on
  mobile, where the sidebar is a sheet), keeps turning and drawing its links
  there, ringed by the link-state bar; the engine tracks the slot every frame,
  so collapsing the sidebar carries it along. The orb leads back to the globe.
- Switching networks keeps the page and drops its selection. Both modes
  share one network view (`useNetworkView`), so the globe and every summary
  agree.
- Route changes are view transitions: the interface layer cross-fades while
  the live globe animates itself underneath.

## First load

A held screen with a 2D loader globe (no three.js) shows from the first paint
until the renderer chunk and the first route's data are in (zustand
`boot-store`; data itself goes through the TanStack Query cache via route
loaders). The WebGL globe then takes over at the same size, view and
longitude, grows into its frame and settles on a continent; the interface
fades in after it.
