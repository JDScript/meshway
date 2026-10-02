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
| Rotation | Nothing on the globe is focused. | Stops on interaction or on focusing a placed node. |

Speed and density are not tied to RTT or traffic. Traffic is not measured;
when it is, flow may map to it. `prefers-reduced-motion` turns off rotation,
flow and breathing; states stay distinguishable by stroke and text.

## Interaction rules

- No layout shift, ever. Feedback that is not part of a component's own shape
  (sign-in failures, read failures, confirmations) goes to a toast; panels
  reserve every slot and only change values.
- Forms are real forms: named fields, `autocomplete`, Enter submits. Text
  fields bound to the URL are uncontrolled and never written during an IME
  composition.
- Floating surfaces use one frosted-glass style (`.glass`); its fill, blur and
  shadow are unlayered so component utilities never show through. An opacity
  or filter on a glass panel's wrapper stops its blur, so surfaces animate on
  the glass element itself.

## Surfaces

The globe is always the canvas; everything else floats over it. Each
surface kind has one slot, so nothing stacks:

| Surface | Where | What |
| --- | --- | --- |
| Chrome | always | network card top left (switcher, counts, link bar; on phones bottom centre), preferences and account top right, the dock |
| Dock | left, under the network card | icon rail on wide screens; on phones a round launcher that unfolds the pages |
| Page panel | left, beside the dock | one page's list (the node list today) |
| Peek | right | the overview's quick look at one node (shadcn Drawer, not modal) |
| Workspace | right of the page panel, covering the globe | one entity in full (a node: tabs, more to come); the globe dims and keeps its place |
| Unplaced | the network card's far end, every page | a round count ringed by states; opens the list of nodes without a sourced location |

- Unplaced nodes are not drawn on the globe (no location source exists yet);
  links to them are listed in the node's surfaces only.
- `features/shell/layout.ts` is the single source of geometry. The globe
  centres (and shrinks if needed) in the space the surfaces leave free, and the
  same numbers are published as CSS variables for class-placed surfaces.
- The shell (`features/shell/network-shell.tsx`) is mounted once for all of a
  network's pages, so changing pages only brings surfaces in and out; it alone
  writes the globe's inputs (lists may highlight a node).
- Surfaces appear in place with a fade and a short shift and keep their last
  content while leaving (`Surface`); the peek slides as a drawer.
- On phones the page panel, the peek and the workspace are glass drawers
  (`GlassDrawer`): swiping one away, Escape, or a tap outside it (not on the
  chrome or its menus) steps back - the list to the overview, the workspace
  to the list.

## URL and state

| State | Owner |
| --- | --- |
| What is open: page, entity, peek, tab, list search and filter | the URL (TanStack Router, Zod-validated; defaults left out) |
| Server data | TanStack Query cache, filled by route loaders |
| Globe inputs | `features/globe/store.ts` (zustand), written by the shell |
| First-load sequence | `features/boot/boot-store.ts` |
| Transient UI (a menu is open) | component state |
| Preferences (theme, locale) | `localStorage` |

Routes: `/networks/<id>/overview?node=` (peek), `/networks/<id>/nodes?q=&state=`
(list), `/networks/<id>/nodes/<node>?q=&state=&tab=` (workspace over the list).
Opening a peek or a workspace from nothing pushes a history entry marked
`opened`; switching entities replaces it; closing an `opened` entry goes back,
so Back and the close button agree. Typing into a search replaces. Switching
networks keeps the page and opens nothing. Every move goes through
`useShellNavigation`.

## Design tokens

Tokens beyond shadcn's are in `src/index.css` (`@theme static` / `@theme
inline`): surface radii (`surface` 22, `sheet` 28) and concentric inner radii
(`row` 14, `tile` 12: inner = outer − inset); type roles (`meta`, `row`,
`lead`, `display`); motion curves (`settle`, `entrance`, `leave`; springs in
`lib/ease.ts`); fills on glass (`fill-subtle`, `-hover`, `-selected`,
`-raised`, `divider`); vitality colours with distinct dot shapes (lit,
solid, hollow). Components use these instead of literal values.

## AgentUI

The dock's hover pill, active pill and the segmented controls follow
AgentUI's animated sidebar and segment patterns
(https://www.agentui.pro). `components/motion/shared-layout-bg.tsx` and
`lib/ease.ts` are its registry source, only reformatted; they need `motion`.

## Globe

- One globe for the whole console, rendered by globe.gl (camera, controls,
  atmosphere, DOM markers) plus our own shader layers (surface, land dots,
  links, pulses, the loader's comet). Colours come only from theme tokens.
- The camera and the offset follow critically damped springs, re-aimed
  whenever surfaces change, so motion never stops dead or restarts.
- The engine chunk is lazy-loaded (~550 kB gzip, mostly three.js and
  three-globe); trimming it is open work.

## First load

A held screen with a 2D loader globe (no three.js) shows from the first paint
until the renderer has drawn and the first route's data are in. Its backdrop
then goes at once (the stage draws the same one); the loader cross-fades into
the WebGL globe in the same pose, comet included, and the globe grows into
its frame and settles on a continent while the interface comes in. View
transitions are not used: during the first load they would lift the interface
above the held screen.

## Development aids

- Alt+G toggles the layout guides (development builds only): every line of
  `layout.ts` and a 4px grid, for checking alignment by eye or in crops.
