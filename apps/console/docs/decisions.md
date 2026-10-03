# Console decisions

The Meshway console: a pure Vite SPA served by the controller. Decisions are
recorded as they are confirmed with the user.

## V2 controller

The console consumes the V2 controller (`apps/meshway-controller`) through
its generated client only, and only what the controller delivers:

- Delivered: registration (when the controller publishes it as open), sign-in,
  session restore and sign-out (`getAuthOptions`, `register`, `login`,
  `getAuthSession`, `logout`); networks (`listNetworks`, `createNetwork`,
  `getNetwork`, `renameNetwork`); members and join links
  (`listNetworkMembers`, `changeMemberRole`, `getMemberRemovalImpact`,
  `removeMember`, `getLeaveImpact`, `leaveNetwork`, `listJoinLinks`,
  `createJoinLink`, `revokeJoinLink`, `inspectJoinLink`, `redeemJoinLink`);
  the network's activity (`listNetworkActivity`); the account (`getAccount`,
  `changeAccountEmail`, `changeAccountPassword`, `listAccountSessions`,
  `revokeAccountSession`, `revokeOtherAccountSessions`,
  `revokeAllAccountSessions`, `listAccountActivity`) and password reset
  links (`inspectPasswordReset`, `redeemPasswordReset`).
- Pages now: `/login`, `/register`; `/` opens the remembered network (kept
  per account) or the first listed, else offers creating the first; a
  network's overview (the empty globe and one line saying this controller
  provides no nodes), members (for `actions.manageMembers`: the members and
  the join links, a member's workspace), settings `network` (the name,
  editable when `actions.rename`; the ID, creation date and role; leaving)
  and `activity` (for `manageMembers`), and the account (email and ID,
  changing the email and the password, the sessions with ending one, the
  others or all, the account's activity). `/join` accepts a join link;
  `/reset-password` takes a reset link the controller's host issued. The
  network menu switches networks (a page at a time) and creates one.
- Members: each member's `actions` alone decide what is offered (a role
  among `assignableRoles`, removal; for oneself removal is leaving). Removal
  and leaving first read their impact (the User-identity devices deleted,
  Tag devices kept) and send its `version` and `revision`; a
  `version_conflict` reads the members again. A role change sends the
  member's `version`.
- Join links: a role and an expiry (1 hour to 7 days); the link
  (`/join#token=…`) is shown once in its dialog, which unmounts on close. A
  repeated request (`secretDelivery: "unavailable"`) says the value cannot
  be read again and offers revoking it and issuing another. The controller
  publishes no roles a viewer may issue or links a viewer may revoke, so all
  roles and every pending link's revoke are offered and a refusal says so.
- `/join` reads the token from the fragment once and replaces the address.
  Signed out, the card signs in (or registers, only while registration is
  open) in place; signed in, it shows the network, role, expiry and account
  (`inspectJoinLink`), and joining opens the network; `already_member`
  opens it too. A session the controller rejects there returns to the
  card's sign-in, keeping the link. The language switch stays disabled
  while the link is held.
- Configuration (`/configuration`, every member; the controller reads it
  to owners and admins only and refuses others, which the page says):
  `getConfiguration`, `getConfigurationSource`, `getConfigurationSchema`,
  `validateConfiguration`, `applyConfiguration` (the `document` and
  `restore` inputs), `listConfigurationRevisions`,
  `getConfigurationRevision`, `compareConfiguration`. The sections read
  only, the whole document as JSONC (`actions.editConfiguration`) and the
  revisions. One draft per sign-in and network (`features/configuration/
  draft.ts`): the source text as typed, invalid or with comments, and the
  revision it was read at; kept across the network's pages, in memory only
  (the source holds secrets: never storage, the URL or the query cache),
  dropped when the sign-in changes. Applying always validates first and
  opens a review: the effective change count, permanent node deletions and
  address changes on their own, every change with its value before and
  after (non-secret views); only its confirmation applies, against the
  draft's revision. A newer revision keeps the draft and its base; the bar
  offers to start over from it or to check the same text against it (a
  new review). A restore takes the same review. The section forms (typed
  `operations` into the same draft) come next.
- Not delivered: nodes, the configuration's section forms, and the event
  stream. Their pages below are the target design; they return when their
  operations exist, and nothing stands in for them. What the console needs
  from the V2 API is in `v2-api-needs.md`.
- What a role may do is the network's `actions`, as the controller computes
  them; the console never derives it from the role.
- Changes the controller deduplicates (`createNetwork`, `renameNetwork`)
  carry a fresh `requestId` per distinct attempt; after an attempt whose
  outcome is unknown (no 4xx answer), the same input sends the same request
  again (`lib/attempt.ts`). A rename sends `expectedName`, the name last
  read; on `name_conflict` the network is read again and the typed name
  kept. A name is checked before sending by the rules the controller
  publishes (`networkNameRules`, `lib/network-name.ts`: raw UTF-8 bytes and
  Cc characters, then Unicode White_Space trimmed and code points counted);
  the controller checks again.
- A network's name is entity metadata, not configuration: it is saved by
  its own button, with no draft or commit bar.
- `lib/api.ts` holds the one `ControlClient` (same origin). `call` unwraps a
  result and throws `ApiFailure`; `authed` adds this tab's session and turns a
  401 into a sign-out. `lib/failure.ts` words a failure by its problem code,
  else by its kind and status. Reads retry only a lost connection or a 5xx.
- The shared components (`components/chrome`, the `components/ui` pieces
  they use, `hooks/`) stay for the pages to come, though no page uses most
  of them yet. Dependencies no page uses are removed; the pages that need
  them re-add the versions they were tested with: `cmdk` ^1.1.1 (the
  palette), `recharts` 3.8.0 (latency history, with the shadcn `chart`).
  The JSONC editor's (CodeMirror 6, `jsonc-parser` 3.3.1) are back with the
  configuration source; the editor chunk loads with the first source
  opened (~111 kB gzip).
- A change the controller makes to every session (changing the password,
  ending all sessions, a reset) ends this tab's too: it forgets the session
  and opens the sign-in with the reason (`/login?reason=`). A 401 that
  refuses a password the call carried (`invalid_credentials`) does not end
  the session: the controller checks the session first.
- A tab signs out when its session reaches the controller's absolute expiry
  (`useSessionExpiry`), not only on its next request.
- Development: start a controller on a private data directory with
  `--listen 127.0.0.1:0`, then run the console with
  `MESHWAY_CONTROLLER_URL=http://127.0.0.1:<port> pnpm dev`. Without it the
  development server does not start, so it never proxies to another
  controller.

## Foundation

| Topic | Decision |
| --- | --- |
| Scope | Product, flows, visuals and code are rebuilt together. The map direction is kept. |
| Stack | Vite, TanStack Router (file routes, typed params and search), TanStack Query, shadcn (Base UI, preset `b0` = `base-nova`, pointer; only the pieces in use are kept, the CLI adds others back), Paraglide (zh, en). |
| BFF | None. The browser calls the controller through the generated `@meshway/control-client` with Zod validation. |
| Origin | One origin. The controller serves the built `dist/` with an `index.html` fallback; development proxies `/api` to the controller named by `MESHWAY_CONTROLLER_URL`. |
| Copy | Every string lives in `messages/{zh,en}.json` (shared) or `messages/<feature>/{zh,en}.json`; zh is the base locale, and `src/messages.test.ts` keeps both in step. Interface text states facts only: no explanations of what the display means or of how it was drawn. |
| Routes | Resources are nested and plural: `/networks/<network_id>/<page>`, e.g. `/networks/<id>/overview?node=<node_id>`. `/networks/<id>` redirects to its overview; `/` to the remembered or first network. Selection and filters live in validated search params. |
| Source layout | `routes/` only validate params and search and mount a feature. `features/<feature>/` holds a feature's page, panel and workspace, its `navigation`, `url`, `queries`, `labels` and pure models, with tests beside them. `components/ui/` are shadcn originals (lightly adjusted), `components/chrome/` our shared components, `hooks/` shared React hooks, `lib/` helpers without React (a hook only beside the store it reads, as in `lib/locale`). |

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
  reloads that tab; drafts are keyed by session. The stored shape is
  versioned (`v: 2`: the controller's session ID, the token, its expiry); a
  session of another shape is ignored.
- Email and password go only to the controller. The email is sent as typed
  (a text field, not `type="email"`, whose browser check is narrower and
  rewrites international domains); the controller normalises it. A new
  password is checked against the rules the controller publishes, in Unicode
  code points and UTF-8 bytes, never `String.length`.
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
stands in for an interface that does not exist yet. "Now" is what the V2
controller supports today; the rest of each row is the target design.

| Page | Now | Panel | Workspaces | Dialogs |
| --- | --- | --- | --- | --- |
| Overview `/overview?node=` | the empty globe, one line: no nodes | none: the globe is its index | none; the node peek; with no node or one, what adding nodes takes | |
| Nodes `/nodes?view=nodes\|tags` | absent | nodes (search, state) or tags | `/nodes/<id>?tab=overview\|links\|transport\|manage`, `/nodes/add`, `/nodes/tags/<tag>` | revoke, enrolment token |
| Members `/members?view=members\|links` | for `manageMembers`: members, join links, a member's role and removal | members or join links | `/members/<membership>` (role, devices, removal) | join link (shown once), removal, revoke |
| Configuration `/configuration` | the sections read only; source (JSONC) and history for owners and admins | the source, the history, the sections with what they hold | `/configuration/<section>` (a form; read only now), `/configuration/source?at=<pointer>`, `/configuration/history?revision=&against=` | the review before applying |
| Settings `/settings` | `network`: name, ID, created, role, leave; `activity` for `manageMembers` | sections | `/settings/<section>`: network, activity, DNS | leave |
| Account `/account` (from the account menu) | email, ID, email and password changes, sessions, activity | none | memberships | end a session, the others, all; change the password |

Outside the shell, one card over the globe like the sign-in: `/login`,
`/register` (offered only while the controller publishes registration as
open), `/` with no network (creating the first), a network not found,
`/join`, and later the password-reset and device-approval pages of the V2
API.

- Configuration is one page because it is one document with one revision:
  one draft, one commit bar, one review.
- Tag owners (`tagsOwners`) are configuration; a node's own tags are its
  identity (`nodes[id].identity`), set in its workspace (`manage`).

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
| Chrome | always | network card top left (switcher, counts, link bar), search, preferences and account top right, the dock; on phones a bottom bar on every page (the launcher, then the card without its link bar) and preferences top right |
| Dock | left, under the network card | icon rail on wide screens; on phones a round launcher at the bottom left, under the thumb, that unfolds the pages upwards |
| Page panel | left, beside the dock | one page's list (the node list today) |
| Peek | right | the overview's quick look at one node (shadcn Drawer, not modal) |
| Workspace | right of the page panel, covering the globe | one entity in full (a node: tabs, more to come); the globe dims and keeps its place |
| Unplaced | the network card's far end, every page | a round count ringed by states; opens the list of nodes without a sourced location |
| First run | the overview only, bottom centre of the room the peek leaves; on phones above the bottom bar | with no node (read, not unread or failed) a card with the three steps and, for owners and admins, Add node; with one, a one-line chip; never dismissed, gone once the condition ends |

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
| A draft and its base revision | `useConfigurationDraft` (`features/configuration/draft.ts`); the dock's marks and the close warning in `features/shell/drafts.ts` |
| Checking, the review, applying, conflicts | `useConfigurationApply` → `useCommit` → `CommitBar` |
| Field validity | the generated schemas, before the controller's own checks |

## Command palette

- ⌘K (Ctrl+K off macOS), or the search button first in the top bar, on a
  network's pages only (not the entry pages). A dialog: a 560px glass panel
  in the top third on wide screens; full width under the top bar on phones,
  above the bottom bar or the keyboard (never a bottom sheet). Escape
  returns focus; a jump hands it to where it lands.
- Groups: pages (layers and sections once something is typed), nodes, tags,
  members, rules (connections, grants, Mesh routes by ID), other networks,
  actions. Only what the role can use; read through the pages' own query
  options, the ones the shell does not hold while the palette is open; no
  endpoint of its own. A group being read or unreadable says so in place.
- Matching is plain substrings over each entry's keywords (name first), as
  the policy pickers match; fuzzy matching would find nearly anything inside
  a hex ID. Six results per group, the rest counted.
- Jumps use `history.jump`: a link on wide screens (a workspace marked
  `opened`), the launcher's move on phones. A rule opens its layer at
  `?rule=` and that row takes focus once (`requestRuleFocus`).

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
7. Editors: the configuration's draft (`useConfigurationDraft`) and
   `useCommit`; the panel marks a row holding a draft with
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
- A scrolling body under a fixed header keeps its spacing from the header
  outside itself (the header's padding, a margin under tabs), with only
  `pt-1` inside for focus rings; so its clip line sits 16px clear of the
  header. Every such body carries `scroll-edge`: once scrolled, its top
  edge fades over 16px instead of slicing rows.
  `scroll-edge` is a scroll-driven animation that never finishes, so it
  never goes on an element Base UI animates out (a tabs panel, a popup, a
  collapsible panel): those wait for their own animations before hiding.
  Scroll inside them instead, as a node's tab `Pane` does.
- The commit bar states one thing: unsaved (plain), a field that will not
  save (`destructive`), the controller moved past the draft's revision
  (`warning`, offering to start over from it or to check the draft against
  it: a new review, never an unreviewed overwrite). Its buttons keep
  minimum widths, so a state change only changes words.
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
(https://www.agentui.pro). `components/chrome/shared-layout-bg.tsx` and
`lib/ease.ts` (its one spring) are its registry source, only reformatted;
they need `motion`.

## Globe

- One globe for the whole console, drawn with three.js directly: a small
  stage (`features/globe/stage.ts`: renderer, camera, orbit controls, the
  sphere, the atmosphere, DOM markers through `CSS2DRenderer`) under our own
  shader layers (surface, land dots, links, pulses, the loader's comet).
  Points of view keep three-globe's conventions (radius 100, altitude in
  globe radii). Colours come only from theme tokens.
- The camera and the offset follow critically damped springs, re-aimed
  whenever surfaces change, so motion never stops dead or restarts.
- The engine chunk is lazy-loaded (~145 kB gzip, almost all three.js).

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
