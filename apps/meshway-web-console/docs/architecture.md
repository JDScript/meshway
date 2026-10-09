# Console architecture

How the console's source is organised and where each kind of state lives.
What the pages do is in `decisions.md`; what the console still needs from
the controller is in `v2-api-needs.md`.

## Source layout

```
src/
  main.tsx        assembly: providers, the router, store ↔ router wiring, window listeners
  routes/         the route tree; a route's own code sits beside it
  api/            the server: client, failures, query options, the live stream
  state/          client state: one zustand vanilla store per concern
  lib/            plain logic and the domain models (no React)
  components/     shared interface
  globe/          the WebGL globe
  hooks/          shared React hooks
tests/            Vitest in Node, mirroring src/
```

### Routes

TanStack Router file routes: the folder tree is the URL tree.

- A route with code of its own is a folder: `route.tsx` defines the route
  (params, search, loader) and its page component; children are files or
  folders beside it. A route without code of its own is a single file.
- What only one route uses sits in its folder, in files and folders whose
  names start with `-` (the router ignores them): `nodes/-node-list.tsx`,
  `nodes/$node/-transport/`. What a subtree shares sits at the subtree's
  root: the network layout's chrome in `networks/$network/-shell/`, and the
  globe/topology pages' selection, map panel and topology canvas in `-map/`.
- Another page's typed moves may be used across pages
  (`nodes/-navigation.ts` opens a node from the overview); nothing else
  private to a route is imported from outside it.

```
routes/
  __root.tsx                 the held first-load screen (-boot/), layout guides, the debug panel (-debug/)
  login.tsx, register.tsx
  join/, reset-password/     one-time links read from the fragment
  authorize-device/$request.tsx
  _authed/route.tsx          needs a session; signs out at its expiry
  _authed/index.tsx          the remembered or first network, or creating one
  _authed/networks/$network/
    route.tsx                the network: its event stream, the shell (-shell/), the globe's scene
    overview/  topology/  nodes/  members/  configuration/  settings/  account/
```

### Shared code, by kind

| Folder | Holds |
| --- | --- |
| `api/client.ts`, `api/failure.ts`, `api/traced-fetch.ts` | The one `ControlClient` (same origin, its requests noted while the debug mode is on); `authed` adds this tab's session and turns a 401 into a rejection; failures worded by problem code. |
| `api/queries/` | Query options and keys per resource. Every reader and the live stream use the same keys. |
| `api/live/` | The network's SSE stream: a pure reducer (`reducer.ts`) and the runtime that applies its effects to the query cache (`stream.ts`), telling the debug mode which reads each event touched, and the controls the debug panel uses. |
| `state/` | Client stores (below). |
| `lib/` | Pure modules: the configuration model (`lib/configuration/`: draft, typed operations, JSONC, schema checks, form problems), nodes (`lib/nodes/status.ts`: a node's one status and its tone), observation, history series, addresses (`ip.ts`), labels for values the controller sends (`state-labels.ts`, `activity.ts`, `roles.ts`, `live-labels.ts`), the debug mode's clock offset and differences (`lib/debug/`). Message tables may live here; React does not. |
| `components/ui/` | shadcn originals, lightly adjusted. |
| `components/chrome/` | Shared presentational pieces (`decisions.md`, Shared components). |
| `components/surface/` | The floating surfaces of a network's pages: `PageFrame`, `Surface`, `GlassDrawer`, the commit bar, the geometry (`layout.ts`), the history rules (`history.ts`) and the page list (`pages.ts`). |
| `components/configuration/` | Configuration editing used by the configuration and nodes pages: the form kit, chips, dialer sets, the commit bar's state (`commit.tsx`), the review. |
| `components/entry/` | The entry card, the sign-in and registration forms, in-card sign-in, the network not-found and failure cards. |
| `components/network/`, `components/history/` | Creating a network, leaving or removing a member; the history window control. |
| `hooks/` | Shared hooks: media queries, viewport, visibility, clocks, sign-out and hand-over, password fields, secret mutations. |

## Rendering

- The root route holds the first-load screen; the globe (`globe/globe-stage.tsx`)
  and the toasts sit outside the router, so navigating never re-creates them.
- The network's route (`$network/route.tsx`) opens the network's event
  stream, draws the shell (network card, dock, toolbar), frames the globe
  and puts the network on it, and holds the configuration's review dialogs.
  `-shell/network-scene.tsx` reads the network once for all its pages
  (`NetworkSceneProvider`: nodes, declared edges with runtime state, the
  planned paths from a selected node, the globe's scene). Its `<Outlet />`
  is the page. The topology canvas (`-map/topology-canvas.tsx`) is mounted
  before the outlet so its nodes can leave from and return to the globe's
  markers; the map pages draw the map panel (`-map/map-panel.tsx`).
- Each page's route renders the page: its panel and its workspace through
  `PageFrame`. A workspace belongs to a child route (`nodes/$node`,
  `members/$member`, `configuration/$view`, `settings/$section`,
  `nodes/add`), but the page draws it from the child's parameters, so
  opening and closing it keeps its motion; the child route only validates.
  Between pages a panel is replaced at once.
- Routes say what they are through `staticData` (`page`, `workspace`); the
  shell reads it to place the chrome and the globe.

## State

| State | Owner |
| --- | --- |
| What is open: page, entity, tab, search | the URL (validated search params; defaults left out) |
| Server data | the TanStack Query cache, filled by loaders and reads, written by the live stream |
| Client state that outlives a component | `state/` stores |
| Transient interface state (a menu open, a field typed) | component state |

Stores are `createStore` from `zustand/vanilla`: the state and its
transitions in the module, React reading through selector hooks
(`useStore(store, selector)`). Functions, timers and request handles stay
out of stores, in the module's runtime. A store that holds per-sign-in
data subscribes to the session store and drops it when the sign-in starts,
ends or changes; window listeners are started once by `main.tsx`.

| Store | Holds |
| --- | --- |
| `session` | This tab's session (a copy of storage); `rejectedAt`, which the router answers by opening the sign-in. Another tab's sign-in reloads this one. |
| `configuration-draft` | One draft per network, in memory only; its transitions are `lib/configuration/draft-state.ts`. The dock's mark and the close warning read it. |
| `configuration-apply` | One check → review → apply flow per network, whichever page started it; the unsettled request is sent again after an unknown outcome. |
| `attempts` | Unsettled requests the controller deduplicates by request ID, per kind and target, past the dialog that sent them (runtime; the debug panel lists them). |
| `live` | Each network's stream connection (connecting, live, paused, stopped); liveness (reads poll while not live) is derived from it. |
| `boot` | The first-load sequence. |
| `globe` | What the globe shows; a click's handler is runtime, not state. |
| `history-layer` | The globe/topology history window, per network. |
| `link-secrets` | The join or reset link's secret, taken from the fragment. |
| `preferences` | Theme and language; `applyPreferences` keeps them on the document. |
| `debug` | Whether the debug mode is on (localStorage) and this tab shows its panel (sessionStorage); the panel's tab and height. |
| `debug-trace` | What the debug mode recorded: each network's stream entries, the requests, each cached read's history, clock samples. Memory only, bounded; afresh when the mode toggles or the sign-in changes. |

## Tests

`pnpm test` runs Vitest in Node (`vitest.config.ts`), no browser. Tests
live in `tests/`, mirroring `src/`, and import modules by their `@/` paths
through their exports only: nothing is exported for a test. They cover the
pure modules, the stores without a DOM, the stream's reducer, the copy
(every message in every locale, defined once, used) and the styles' tokens.
Interfaces are checked by hand against a real controller.

## Adding a page

1. A folder under `routes/_authed/networks/$network/<page>/` with
   `route.tsx`: `staticData: { page }`, search validated in `-search.ts`,
   the page component drawing `PageFrame`. Workspaces are child routes
   with `staticData: { workspace: true }`, drawn by the page.
2. Its moves in `-navigation.ts`, built on `useSurfaceHistory`
   (`components/surface/history.ts`).
3. The page in `components/surface/pages.ts` and, if it is in the dock,
   in its list there.
4. Reads in `api/queries/<resource>.ts`; what the live stream carries is
   written there by `api/live/stream.ts` under the same keys.
5. Copy in `messages/<area>/{zh,en}.json`, keys prefixed by the area;
   words several areas share in `messages/{zh,en}.json`.

## Development

Start a controller on a private data directory with
`--listen 127.0.0.1:0`, then run the console with
`MESHWAY_CONTROLLER_URL=http://127.0.0.1:<port> pnpm dev`. Without it the
development server does not start, so it never proxies to another
controller. `pnpm run check` (types, lint, format), `pnpm test` and
`pnpm run build` are what `mise run check` runs for the console.
