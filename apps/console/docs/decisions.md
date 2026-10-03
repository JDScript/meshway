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

## Pages

Only what the controller can do has an entry: no switch, field or page
stands in for an interface that does not exist yet.

| Page | Panel | Workspaces | Dialogs |
| --- | --- | --- | --- |
| Overview `/overview?node=` | none: the globe is its index | none; the node peek | |
| Nodes `/nodes?view=nodes\|tags` | nodes (search, state) or tags | `/nodes/<id>?tab=overview\|links\|transport\|manage`, `/nodes/add`, `/nodes/tags/<tag>` | revoke, enrolment token |
| Members `/members?view=members\|invitations` | members or invitations | `/members/<id>` (role, devices, removal) | invite (link shown once), removal |
| Policy `/policy` | the four layers with their diagnostics | `/policy/<layer>`: connections, grants, mesh routes, egress (public rules, profiles, automatic, assignments), source (JSONC) | |
| Settings `/settings` | sections | `/settings/<section>`: network, DNS | |
| Account `/account` (from the account menu) | none | memberships, password, sessions | |

Outside the shell, one card over the globe like the sign-in: `/login`,
`/invite`, `/reset-password`, `/setup` (instance), `/authorize-device`
(path unchanged for the App).

- Policy is one page because the policy is one document with one revision:
  one draft, one commit bar. Egress is shown but not editable while the
  controller refuses it.
- Tag definitions sit beside the nodes they group; a node's own tags are
  set in its workspace (`manage`).
- The old hand-made connections (`ConnectionPlan`) are not in the console;
  managed listeners and the policy's connection layer replace them.

## Nesting

| Level | What | Rule |
| --- | --- | --- |
| Canvas | the globe | always |
| Chrome | network card, toolbar, dock | always |
| Page panel | a page's index | one at a time |
| Workspace | one entity or one editor in full | one at a time; the deepest level |
| Dialog | an irreversible confirmation, a secret shown once, a very short create | never in the URL |

- Opening another thing at the same level replaces; a reference to another
  page (a node in a policy rule) goes to that page's workspace and Back
  returns. Detail below a workspace is tabs, inline expansion or a split
  inside it, never a second workspace.
- Secrets (enrolment tokens, invitation and reset links) never enter the
  URL, the history or the query cache.
- Editing happens in place. Drafts live in memory per sign-in and network,
  with the revision they were loaded at; polling never moves them. One
  floating commit bar (bottom centre) serves every editor; a 409 or 422
  keeps the draft. The dock marks pages holding a draft; closing or
  reloading the tab with one asks first; signing out drops them.
- Members read; owners and admins change (the controller's `Manage`).
  Controls a role cannot use are not offered; the controller still decides.
- Pages own their surfaces (`features/<page>/page.tsx`) through
  `PageFrame`, which frames them as surfaces on wide screens and drawers on
  phones; the shell only places the chrome and drives the globe.

## Surfaces

The globe is always the canvas; everything else floats over it. Each
surface kind has one slot, so nothing stacks:

| Surface | Where | What |
| --- | --- | --- |
| Chrome | always | network card top left (switcher, counts, link bar), preferences and account top right, the dock; on phones a bottom bar on every page (the launcher, then the card without its link bar) and preferences top right |
| Dock | left, under the network card | icon rail on wide screens; on phones a round launcher at the bottom left, under the thumb, that unfolds the pages upwards |
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
  (`GlassDrawer`) floating above the bottom bar: swiping one away, Escape,
  or a tap outside it (not on the chrome or its menus) steps back - the
  list to the overview, the workspace to the list.

## URL and state

| State | Owner |
| --- | --- |
| What is open: page, entity, peek, tab, list search and filter | the URL (TanStack Router, Zod-validated; defaults left out) |
| Server data | TanStack Query cache, filled by route loaders |
| Globe inputs | `features/globe/store.ts` (zustand), written by the shell |
| First-load sequence | `features/boot/boot-store.ts` |
| Transient UI (a menu is open) | component state |
| Preferences (theme, locale) | `localStorage` |

Each page states its routes in `features/<page>/url.ts` and its moves in
`features/<page>/navigation.ts`, built on `useSurfaceHistory`
(`features/shell/history.ts`): opening a peek, a workspace or a page sheet
from nothing pushes a history entry marked `opened`; switching entities
replaces it; closing an `opened` entry goes back, so Back and the close
button agree. Typing into a search replaces. Switching networks keeps the
page and opens nothing. Routes say what they are through `staticData`
(`page`, `workspace`), which the shell reads to place the chrome and the
globe; `features/shell/pages.ts` lists the pages and the dock.

| Draft state | Owner |
| --- | --- |
| A draft and its base revision | `createDraft` (`features/shell/drafts.ts`), one per resource |
| Saving it, conflicts, the bar | `useDraftCommit` → `CommitBar` |
| Field validity | the generated schemas, before the controller's own checks |

## Adding a page

1. `features/<page>/url.ts` (search schema, sections, tabs),
   `navigation.ts` (the route read with typed `useMatch(…, shouldThrow:
   false)`, the moves on `useSurfaceHistory`), `page.tsx` (`PageFrame`).
   Surface content takes the URL state as props: it stays on screen while
   it leaves, after its route has stopped matching.
2. Routes under `routes/_authed/networks/$network/<page>…`, with
   `staticData: { page }` on the page and `{ workspace: true }` on each
   workspace.
3. List it in `features/shell/pages.ts` (and the dock) and mount it in
   `network-shell.tsx`.
4. Copy in `messages/<feature>/{zh,en}.json`, keys prefixed by the feature.
5. The panel: `PanelHeader` (title, the count of what it names, one
   `PanelAction`) over a `PanelList` of `ListRow`s, `useListKeys` for the
   arrow keys, `ListPlaceholders` / `ListNote` while unread or empty.
6. The workspace: `SurfaceHeader size="lg"` with a `RevisionLine` under an
   editor's title; content in `Section`s, `max-w-workspace` unless it is a
   data-dense view; `EmptyLine` for an empty or unread section; `RefRow`
   for anything with a page of its own; `DangerRow` before an
   irreversible action.
7. Editors: `createDraft` (`createKeyedDraft` for many of one kind) and
   `useDraftCommit`; the panel marks a row holding a draft with
   `DraftMark`; members get the same view, read-only (`useCanManage`).
8. Secrets: `useSecretMutation`, shown in a `GlassDialog` with
   `SecretField` (`CopyField` for a command to copy). Irreversible actions:
   `ConfirmDialog` with its `effects`.
9. Popups inside the workspace body: wrap the body in `PopupBoundary` and
   pass `usePopupPlacement()` to the popup, so it stays below the header
   and clear of the commit bar.

## Shared components

One implementation each, in `components/chrome` (and `features/shell` for
what knows the shell's geometry):

| Piece | What it fixes |
| --- | --- |
| `PanelHeader`, `PanelAction` | One 28px title row on `text-lead`; the count counts what the title names (per-view counts sit in the segment, `SegmentCount`); one action, an `IconAction` whose glyph meets the content edge. |
| `PanelList`, `ListRow`, `StaticRow`, `ListPlaceholders`, `ListNote`, `useListKeys` | Panel rows 56px high, 2px apart (a hovered row never merges with the chosen one), a 16px lead slot (a status dot, a glyph, or a 16px initial avatar, `InitialAvatar`) so every list's text starts at the same x; one trailing column; no chevrons. |
| `RefRow`, `RefList`, `RefStateHeading` | A row in a workspace naming a node or a network: its fill on the section's edges like a tile, the state in a fixed column under its heading, ↗ (or an `IconAction`) at its end. `NodeRef` is the node's. |
| `Section`, `EmptyLine`, `DangerRow` | A titled part with its count and action on a 32px line; the fixed-height empty or unread line (short facts: "没有节点"); the tile before an irreversible action, the fact it stands on and the action. |
| `ConfirmDialog` | `effects`: what it does, one fact each, as a list. `tone`: `destructive` (takes something away) or `default` (cannot be reversed here, takes nothing). |
| `CopyField`, `SecretField` | A value to copy; a command breaks at spaces and flags, only a secret anywhere. `SecretField` marks it as shown once. |
| `IconAction` | An action as a round icon named by its tooltip and its label. |
| `FieldInput`, `FieldSelect` | One fill (`field`, on both themes and when disabled), corners and ring; sizes `default` (workspace, dialog) and `lg` (entry cards). |
| `DraftMark`, `RevisionLine` | Unsaved edits in the one draft colour: a dot beside a title or the word; the editor's `r42`, or the draft and the revision it started from. |

- Workspaces holding a form (a tag, a member, adding a node, the account,
  settings) are `max-w-workspace` (40rem) wide; data-dense views (a node's
  tabs, the policy layers) take the full width.
- The commit bar states one thing: unsaved (plain), a field that will not
  save (`destructive`), the controller moved past the draft's revision
  (`warning`, offering to overwrite). Its buttons keep minimum widths, so a
  state change only changes words.
- Entry pages (`EntryPage`, `EntryCard`, `EntrySubmit`) share one 380px
  card and one hand-off (`HANDOFF_MS`); while a secret is held the
  language switch stays in place, disabled.

## Design tokens

Tokens beyond shadcn's are in `src/index.css` (`@theme static` / `@theme
inline`): surface radii (`surface` 22, `sheet` 28) and concentric inner radii
(`row` 14, `tile` 12: inner = outer − inset); type roles (`meta`, `row`,
`lead`, `display`); motion curves (`settle`, `entrance`, `leave`; springs in
`lib/ease.ts`); fills on glass (`fill-subtle`, `-hover`, `-selected`,
`-raised`, `-tint`, `divider`); `draft`, the one colour of unsaved
edits; the workspace measure (`--container-workspace`, `max-w-workspace`,
40rem); vitality colours with distinct dot shapes (lit, solid, hollow).
Components use these instead of literal values.

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
