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
  links (`inspectPasswordReset`, `redeemPasswordReset`); nodes
  (`listNodes`, `getNodeSettings`, `deleteNode`, `revokeNodeKey`), enrolment keys
  (`listEnrollmentKeys`, `createEnrollmentKey`, `revokeEnrollmentKey`) and
  the transport catalog (`getTransportCatalog`); the network's State
  (`getNetworkState`).
- Pages now: `/login`, `/register`; `/` opens the remembered network (kept
  per account) or the first listed, else offers creating the first; a
  network's overview (the globe with the State's located nodes and links,
  the status corner and the history layer), nodes, members (for `actions.manageMembers`: the members and
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
  be read again and offers revoking it and issuing another. The roles
  offered are `NetworkActions.joinLinkRoles` (none: no issuing); revoking is
  offered where `JoinLinkView.actions.revoke`. The list is the controller's
  order (newest first) and its count the controller's `total`.
- `/join` reads the token from the fragment once and replaces the address.
  Signed out, the card signs in (or registers, only while registration is
  open) in place; signed in, it shows the network, role, expiry and account
  (`inspectJoinLink`), and joining opens the network. A used, revoked or
  expired link, or one whose issuer may no longer invite, says which; an
  unknown token says only that the link is not valid. `alreadyMember` (or
  `already_member` on joining) offers opening the network instead. A session the controller rejects there returns to the
  card's sign-in, keeping the link. The language switch stays disabled
  while the link is held.
- Configuration (`/configuration`, every member; the controller reads it
  to owners and admins only and refuses others, which the page says):
  `getConfiguration`, `getConfigurationSource`, `getConfigurationSchema`,
  `validateConfiguration`, `applyConfiguration` (the `document` and
  `restore` inputs), `listConfigurationRevisions`,
  `getConfigurationRevision`, `compareConfiguration`. The whole document as
  JSONC (`actions.editConfiguration`), the revisions, and forms for access,
  topology (with `defaults.topology`), traffic routing, rule sets, groups,
  tag owners, DNS, address pools and the defaults (access, topology,
  dialers); "nodes" lists each node's settings and opens its workspace,
  where they are edited. One
  draft per sign-in and network (`features/configuration/draft.ts`), kept
  across the network's pages, in memory only (the source holds secrets:
  never storage, the URL or the query cache), dropped when the sign-in
  changes. It holds one kind of input with the revision it was read at:
  the source text as typed (invalid or with comments), or the forms' typed
  operations over the non-secret view of that revision, one `replace…` /
  `setAddressPools` per edited part holding its final value (a part set
  back to its base value drops its operation), and per node one operation
  per target (`features/configuration/node-operations.ts`): settings with
  only the fields that differ, tags, addresses with only the families that
  differ, each listener (set, removed, or recreated: removed then set, so
  the controller makes new material), and `deleteNode`, which replaces the
  node's other operations. Never both, and one is never
  rebuilt from the other: a form reads only while the source holds edits,
  and the source while the forms do; each says so and offers to apply or
  discard first. Values are checked in the form the configuration
  reference states before applying (the bar says how many fields will not
  save); the controller's refusals sit under the rules and fields at their
  pointers. One `replaceDefaults` holds the topology default and the
  dialer defaults, with only the fields that differ from the base (the
  controller keeps what is omitted), so changing the topology default
  alone sends only it, and checking against a newer revision replays only
  it. The commit bar and review serve the configuration page
  and the nodes page alike (`useConfigurationCommit`).
  Applying always validates first and opens a review: the effective change
  count, permanent node deletions and address changes on their own, every
  change with its value before and after (non-secret views); only its
  confirmation applies, against the draft's revision. A newer revision
  keeps the draft and its base; the bar offers to start over from it or to
  check the same text or operations against it (a new review). A restore
  takes the same review. An apply's request ID is reused only to repeat
  the same proposal after an unknown outcome; `request_id_conflict` (that
  ID with other input) and `revision_conflict` are worded apart. An apply
  with no target nodes (`targetCount: 0`, `publicationIds: []`) says no
  deployment is needed.
- Nodes (`/nodes?q=`, every member: the controller lists owners and admins
  every node, a member their own User-identity devices). The list searches
  by hostname and shows each node's identity (its user's email or its
  tags), first address, OS and Agent version when reported, whether it is
  disabled or has a revoked key, else its report: a lit dot and when it
  arrived while the controller calls it current, its freshness otherwise.
  The State is the network's (`getNetworkState`, one read for every node).
  A node's workspace (`/nodes/<id>?tab=overview|settings|transport|latency|manage`):
  - overview: the registration facts; the report's freshness, times
    (received, observed, expiry, last authenticated) and instance; from a
    full report the system (reported hostname against the configured one,
    OS, architecture, Agent version, routing profile), the configuration
    saved / applied / in progress against the current revision, and the
    problems the node reports; the inventory's `desiredExecution` (the
    target the controller wants it to run: revision, publication, digest),
    never itself saved or applied, said against the report's saved and
    applied (same target applied, only saved, another target, no report); the controller's location (or why it has
    none) and IP information.
  - settings: hostname, remark, enabled, exit, tags (a User identity given
    tags becomes a Tag identity; the last tag cannot be taken), Mesh
    addresses within the pools (empty: no address of that family), and the
    dialers: inherit the defaults (shown), a set of its own, or none (`{}`).
  - transport: each configured listener with what the report says of it
    (phase, problem, loaded certificate, sockets and their endpoint
    candidates by origin, observed and expiry, expired ones marked), other
    sockets, listeners reported but not configured, the node's links
    (peer, evidence, transport, who dialed, business use, since, phase)
    and paths. A listener is edited inline and enters the draft only when
    valid; it restates the view's other fields; secrets are write-only (the
    UUID and a provided certificate are kept unless written; recreating
    generates new ones). Trojan listeners and listeners with WebSocket
    response headers (shown by name only) are edited in the source.
  - manage: the keys with their creation times, each revoked (with
    `expectedGeneration`) where `canRevokeControlKey` /
    `canRevokeDataplaneKey`; a `version_conflict` reads the nodes again
    and keeps the confirmation over the key read; and permanent deletion
    where `canDelete`. With form edits in the draft, deletion
    joins it as `deleteNode` (reviewed with the rest, listed as permanent);
    with source edits it is refused; with no draft it is sent at once,
    against the revision read when it is asked (a `revision_conflict`
    reads it again and keeps the confirmation open).
  Without `actions.editConfiguration` the settings read only; a member's
  come from the event stream (`memberConfigurationQuery`) once it has sent
  them, else from `getNodeSettings` (`nodeSettingsQuery`, key
  `[network, "nodes", id, "settings"]`, polled while the stream is not
  live).
- Adding a node (`/nodes/add`, for `actions.editConfiguration`): an
  enrolment key for a Tag-identity device: at least one tag among the
  configuration's `tagsOwners`, an optional hostname (one lowercase DNS
  label; empty takes the device's), an expiry (1 hour to 30 days), and
  at most one listener (none, UDP, TCP, VLESS, VLESS · WS among what the
  transport catalog offers): the port, an optional public address, for
  VLESS a TLS name (else the public address), and under "advanced" the
  bind address and a public port of its own. Only what is typed is sent;
  the controller fills its defaults and generates credentials. The
  controller checks the listener only when the node enrols, so the form
  checks the generated `Listener` schema and the host rules first. The
  key is shown once in a dialog dropped when it closes; this controller
  provides no Agent install, and the page says so instead of a command.
  Below, the keys (`listEnrollmentKeys`, newest first, a page at a time
  with the controller's total): each key's tags, the initial hostname
  (else the device's) and listeners (the non-secret view), who created it
  (by email while the members are readable, else the user ID), and its
  state from its fields: used (with when, and the node it became, opened
  from there while listed), else revoked, else expired, else unused.
  Revoking is offered where `actions.revoke`; the controller answers a key
  used or expired first alike, so the list read again says which.
- `/authorize-device/<enrollmentRequestId>` approves a device's enrolment
  request (`getDeviceAuthorization`, `approveDeviceAuthorization`,
  `denyDeviceAuthorization`). The path is the console's: neither the
  controller nor docs/v2 defines the browser link a device opens yet. The
  request ID is not a credential; the page signs in in place as `/join`
  does (`entry/inline-sign-in.tsx`), shows what the device reported as
  untrusted text with its control key, and follows the request until it
  settles. A pending request has no user and is never denied: it is
  approved into one of the account's networks, or for a re-login
  (`existing`: the node signing in again, which then belongs to this
  account) only into the node's network; "not now" only leaves the page
  (the device cancels, or the request expires). Approved, the page shows
  the `approval` when the controller gives it (by this account, else by
  the member's email where this account manages the network, else the
  user ID) and, by `returnMethod`, to go back to the device; only the
  approver withdraws it (`denyDeviceAuthorization`) before the device
  redeems it. `withdrawn` and `cancelled` are told apart; a
  `state_conflict` reads the request again.
- The event stream (`watchNetwork`, `features/live/`): SSE first, GET as
  the fallback. Each tab keeps one stream for the network it has open, from
  its pages' mount until the network changes or the sign-in ends. The
  snapshot batch is staged and installed at `sync.ready` in one step; then
  each delta must continue from the last applied cursor, else the view is
  dropped and synced afresh. It writes only the reads it carries exactly:
  State (`state.snapshot` / `state.delta`), nodes (entities, ordered by ID
  as `listNodes`), the network summary (inside the read `getNetwork`
  response), the current revision's progress and a member's own node
  settings (`memberConfigurationQuery`). Members, join links, activity,
  configuration reads and rule sets are invalidated instead (paged, or a
  GET adds what the event lacks); `configuration.changed` never touches a
  draft, which sees the new revision through those reads. A member's
  configuration change takes a new snapshot (no event carries their
  settings). Reconnects back off (1 s doubling to 30 s), resuming from the
  applied cursor; `sync.reset` and a refused frame sync afresh. 401 signs
  out; 403/404 stops the stream and the network's reads say why. While it
  is not live, the reads it covers poll (`pollUnlessLive`), and leaving
  live reads them again. The toolbar's dot states it (live since,
  reconnecting since, stopped). Observation events write this sign-in's
  observation reads (below); a new sync reads them again.
- A revision's deployment (`getConfigurationProgress`), in every
  revision's history entry: the controller's summary (targets once each,
  pending / saved / applied / failed), never recomputed here, and each
  target node's phase, freshness and last report time; `partial` scope says
  only visible nodes count. A historical "applied" is not current health.
  Another revision than the current one is read again every 3 s while shown.
- Traffic routing (`replaceRouting`) and rule sets (`replaceRuleSets`) are
  forms like the others. Ordered lists (Mesh routes, public and profile
  egress rules, the automatic profile's regions) are numbered on a rail
  ending in what happens when none matched, and move with their arrows or
  Alt+↑/↓; via nodes and exits are concrete nodes in priority order, an
  exit not `exitEnabled` is marked; profiles name the built-ins (Mesh
  only, `global:<exit>` per exit node, automatic when configured) and the
  configuration's own. A match is one chip set: domain suffixes, prefixes
  and rule sets (`set:<key>`). The controller reports no shadowed rules,
  so none are shown. Rule sets: inline content, or a remote HTTPS source
  with its format and interval. A remote set that is new or has a new
  source (URL or format; the interval alone is not) is downloaded and
  converted by the controller when the draft is checked and again when it
  is applied, so one apply adds a set and the rules naming it; the review
  lists `preparedRuleSets` (entries, digest) as tentative, nothing saved.
  A failed preparation comes back as an issue at the set, shown there and
  under the rules naming it. The status (`listRuleSets`, Owner / Admin, key
  `[network, "rule-sets"]`: content, checked, last success, problem,
  publication) is for the current revision's definition; a set whose
  source is saved as the draft has it can be refreshed (`refreshRuleSet`),
  and the controller checks it on its `updateInterval`. A set without
  content is never treated as empty. Renaming a set or a profile renames
  its references.
- Overview (`getNetworkState`, `listNodes` for names,
  `queryObservationHistory`): a node is on the globe only with the
  controller's `derived.location` and its coordinates; pointing at one
  shows the place, the address it rests on and its source (STUN or
  interface), observation and expiry, and the dataset. Every other node is
  listed under 待定位 with the controller's `locationAbsent` (a GeoIP result
  without coordinates names its region). Current links (`StateView.links`)
  are drawn only between located ends, styled by evidence, then phase
  (matched, one end, conflicting, expired, preparing, failed, closed), and
  never animated: nothing travels along a line, and `inUse` is a listed
  fact. Links with an unlocated end are counted and listed, not drawn. Links
  between the same nodes take successive arcs. The corner counts nodes by
  freshness and links by phase and evidence, with the State's `asOf` and
  whether the stream keeps it live. A node opens its workspace from the
  globe or the list. More than 120 placed nodes show names only on the one
  pointed at, focused or selected.
- History layer (D3): off by default; a chosen window (1 h, 6 h, 24 h, 7 d,
  the controller's longest) is fixed when chosen, and refreshing reads a new
  one, because cursors are bound to the window and lapse after 15 minutes.
  `recentLinks` (kinds `link_opened`, `link_closed`), both ends' records of
  a link merged, the latest deciding opened or closed; drawn faint, apart
  from current links, and never for a link that is current. The first 5
  pages (200 each) are read on their own, then on request. Coverage is
  shown as the controller states it: pruning that reaches into the window,
  and each collection gap with its own interval and drop count.
- Node latency (`?tab=latency`): `buckets` of `probe_completed`, sent by the
  node (`sourceNodeId`) or to it (`peerNodeId`), in the same windows (1 min,
  5 min, 15 min, 1 h buckets). One series per source, probe kind, path, link
  and transport; the controller's per-instance and per-publication buckets
  of one series merge into one point (the mean weighted by successes, exact
  because a success always has its duration). A step per bucket for the
  mean of successful probes, a strip for the buckets with records, warning
  where probes timed out or failed; a bucket with no records is blank and
  one with no success has no step, never zero. The window's mean, bucket
  mean range, probe counts and 未回应率 (timed out or failed over probes;
  not packet loss). Drawn with SVG by hand. The history reads records
  only; it never starts a measurement.
- Live observation (above the node's latency history; `features/observation/`):
  `createObservation`, `getObservation`, `listObservations`,
  `renewObservation`, `cancelObservation`, and the stream's
  `observation.status`, `observation.sample`, `observation.removed`. Offered
  from the node to owners and admins, and to a member for their own
  User-identity device (the controller publishes no action for it, so its
  rule is restated in `choices.ts`; it still decides). A target among the
  nodes this account sees, the exact service (ICMP echo, TCP or UDP with a
  port), automatic or explicit path, a period of 1 to 30 s; it asks for
  `rtt` and `probeLoss` (traffic is not offered: the controller returns it
  unsupported). An explicit path is one the network has shown: the direct
  one, the State's reported paths between the two, or one an observation of
  this sign-in took; the controller publishes no allowed list. Each distinct
  attempt has a fresh `subscriptionId`; an unknown outcome sends the same
  again. A tab holds one subscription: its phase, selected path and segment,
  lease and renewal, the controller's `unsupportedMetrics`, and the samples
  it saw arrive (`[network, "observations", "subscription", id]`; the
  controller sends only the latest, so a reload or take-over starts from
  that one). Timed replies are points, joined while they share a path
  segment and none is more than two periods apart; timeouts, unsent probes
  and open windows are marks with no value; time without a sample is blank;
  counts and 探测未回应率 (timed out over replied plus timed out; unsent
  probes apart). Renewal is at `renewAfter` (a little after, by this
  browser's clock), only while the tab is visible; a renewal refused as
  `observation_limit` is tried again each second while the lease lasts. A
  lease past `subscriberExpiresAt` shows as ended and is read again.
  Closing the panel or leaving the tab (another node tab, network or page)
  cancels; closing the browser leaves the lease to run out. This sign-in's
  other active subscriptions from the node (`listObservations`, another
  tab's or one from before a reload) can be taken over or cancelled.
  Refusals are worded by code for the call (403, 409, 429, 404). Nothing is
  measured by the console, and nothing is shown that the controller did not
  send.
- Not delivered by the controller: GeoIP data (no node has a location
  yet), the install script and Agent releases; nothing stands in for them.
  What the console needs from the V2 API is in `v2-api-needs.md`.
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
  palette).
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
| Overview `/overview?node=` | the globe with located nodes and current links; the status corner (counts, 待定位, current links, the history layer) | none: the globe is its index | none; the node peek; with no node or one, what adding nodes takes | |
| Nodes `/nodes?view=nodes\|tags` | `?q=` with each node's report; a node's facts and State, settings, listeners with their runtime, links and paths, keys and deletion; adding one with an enrolment key | nodes (search, state) or tags | `/nodes/<id>?tab=overview\|settings\|transport\|latency\|manage`, `/nodes/add`, `/nodes/tags/<tag>` | revoke, enrolment token |
| Members `/members?view=members\|links` | for `manageMembers`: members, join links, a member's role and removal | members or join links | `/members/<membership>` (role, devices, removal) | join link (shown once), removal, revoke |
| Configuration `/configuration` | for owners and admins: the source (JSONC), history, and forms for access, topology, routing, rule sets, groups, tag owners, DNS and pools (the other sections read only) | the source, the history, the sections with what they hold | `/configuration/<section>` (a form, or the section read only), `/configuration/source?at=<pointer>`, `/configuration/history?revision=&against=` | the review before applying |
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
