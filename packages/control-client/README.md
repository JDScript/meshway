# Control client

The Controller's actual Rust routes and `meshway-protocol` types are the source
for the OpenAPI document, TypeScript types, Zod schemas and operations. Generated
files must be updated with `generate`, never edited directly.

## TypeScript

```ts
import { ControlClient, login, getAuthSession } from "@meshway/control-client";

const client = new ControlClient({ baseUrl: "https://controller.example" });
const result = await login(client, { email, password }, { signal });
if (result.ok) {
  const current = await getAuthSession(client, {
    token: result.data.token,
    signal,
  });
} else if (result.error.kind === "http") {
  // HTTP status is retained even if the error body cannot be decoded.
  const code = result.error.problem?.problem.code;
}
```

- The package root exports `ControlClient`, request options/result types, DTO
  types and the account operations: `getAuthOptions`, `register`, `login`,
  `getAuthSession`, `logout`. Network operations are `createNetwork`,
  `listNetworks`, `getNetwork`, `renameNetwork`, and `listNetworkMembers`.
  It also exports the implemented account/session management, membership/join-link,
  configuration/edit/validation/history and activity operations. The complete
  generated operation index is `src/generated/operations.gen.ts`.
- `@meshway/control-client/schemas` exports `z<TypeName>` validators.
  `@meshway/control-client/json-schema` exports editor schemas.
- All bodies are validated, including duplicate decoded JSON keys in nested
  objects. An invalid success body yields `decode`; an invalid
  error body yields `http` with its status and no `problem` or raw body.
- Other failure kinds are `configuration`, `request`, `network`, `body_limit`
  and `cancelled`. A network read failure retains the status when available.
- Activity actions use dotted wire names such as `session.login` and
  `membership.role`; the generated enum and runtime validators use those names.
- Incorrect current passwords return `400 invalid_current_password`, distinct
  from `401 invalid_session`. Pagination uses `400 invalid_cursor` consistently.
  Reset errors are `invalid_password_reset`, `password_reset_used`,
  `password_reset_expired`, or `password_reset_replaced` (all HTTP 400).
- Session `lastUsedAt` records authenticated use with 30-second write coalescing;
  it does not extend the session. `clientLabel` is a coarse product/platform
  summary of the issuing client's User-Agent, or null when unknown.
- `baseUrl` is an HTTP(S) origin, without path, query, fragment or credentials.
  The caller supplies session tokens per authenticated call and owns storage,
  logout and cache lifetimes. The SDK has no browser persistence or BFF.
- `fetch` can be injected. Redirects and ambient cookie credentials are disabled.
  `AbortSignal` cancels pending sends/reads. The default body budget is 1 MiB;
  `maxBodyBytes` accepts 1 through 16 MiB.

## Networks

Network methods take generated path/query objects. For example:

```ts
const created = await createNetwork(client, { requestId, name }, { token, signal });
const page = await listNetworks(client, { limit: 100 }, { token, signal });
const network = await getNetwork(client, { networkId }, { token, signal });
```

Creation is idempotent per user and request ID; it atomically stores the network,
Owner membership, complete encrypted initial configuration, and activity.
Rename takes `expectedName` and does not change the configuration revision.
Pagination uses an opaque, expiring cursor bound to the caller and query scope;
metadata and permissions are checked again on every page. Only network
Owner/Admin can read the management member list in this slice.

Both list responses include `total`: the current visible count within the
cursor's original creation boundary, recalculated on every page. Newly created
objects do not enter an older traversal; removed or newly hidden objects can
reduce its total. Each member includes `actions.changeRole`,
`actions.assignableRoles`, and `actions.remove`. Mutations recheck permissions,
last-Owner constraints and `expectedVersion` in the transaction.

`getAuthOptions().networkNameRules` supplies the server's shared name rules:
raw UTF-8 at most 1024 bytes, no Unicode Cc characters, then Unicode White_Space
edge trimming and 1–128 code points. U+FEFF is not trimmed. Do not use raw
`maxLength=128`, UTF-16 `String.length`, or JavaScript's different `trim()` rules.

## Configuration

`getConfiguration` returns the redacted view. `getConfigurationSource` is an
explicit secret read of the full JSONC and complete protocol values. Use
`validateConfiguration` for the effective view, change count, affected resources
and explicit `unavailableDependencies`, then `applyConfiguration` with the same
`expectedRevision` and a request ID. Inputs are the generated `ConfigurationRequest` union:

- `document`: complete `sourceJsonc`.
- `operations`: typed `ops`; `setListener` requires an explicit `listener`
  update value, with `null` meaning removal. Updates to the same listener retain
  omitted protocol/TLS secrets. Dialer WebSocket headers support omission to
  keep values, an explicit map to replace them, and `{}` to clear them. Protocol
  sets and non-secret fields still follow their declared replacement/default rules.
- `restore`: `restoreFrom`, committed as a new revision after revalidation.

Strict CAS rejects stale edits. A request ID can recover an already committed
identical apply. `compareConfiguration` and `listConfigurationRevisions` read
actual complete versions; they never return protocol secrets in the diff.
Document and operations inputs preserve existing stable reference bindings;
an unchanged save does not reauthorize a removed/rejoined membership. Retained
inactive references are reported as unavailable dependencies, and permitted
binding changes participate in the preview's effective changes.

Node-dependent configuration/member removal now uses the actual execution
participant and permanent revocation transaction. Remote rule-set publication
still needs its resource provider. Address reassignment and transport material
rotation are not exported operations yet.

Management success is not evidence that a Node has installed configuration or
established a tunnel.

## Rust

`meshway-control-client` exposes `Client` and `ClientError<ErrorResponse>`.
Generated inherent methods use snake_case operation names and reuse
`meshway_protocol` DTOs directly. Password/token fields use `WireSecret`; access
to their string value is explicit through `expose()`.

```rust,ignore
let client = meshway_control_client::Client::new(
    "https://controller.example",
    protected_http_builder, // reqwest::ClientBuilder supplied by the caller
    1024 * 1024,
)?
.with_middleware(caller_middleware);
let issued = client.login(&meshway_protocol::LoginRequest {
    email,
    password: meshway_protocol::WireSecret::new(password),
}).await?;
let session = client.get_auth_session(issued.token.expose()).await?;
```

The SDK sets `Policy::none()` before building the supplied HTTP builder, retaining
its resolver/socket configuration. Configure timeouts on that builder. Dropping
an operation future cancels its send/read; the runtime spawns no detached request
tasks. Error Debug/Display omits decoded problems and all raw transport content.

## Node control operations

Generated operations cover signed device enrollment/PKCE, enrollment keys,
challenge/session authentication, key rotation, instances/leases, execution
configuration reads, demands, receipts, inventory and permanent deletion.
`deleteNode` uses authenticated DELETE with its typed CAS/request-ID JSON body.
The ordinary account registration/login operations do not substitute for a
Node's proof of private-key possession.

`getDeviceAuthorization` and `approveDeviceAuthorization` return
`DeviceAuthorizationView { request, approval }`. `request` includes the existing
binding and explicit `returnMethod: "poll"`; approval is disclosed only to its
approver or a current manager of that network. Device-side signed query can read
its approval. Terminal phases distinguish `cancelled` (device) from `withdrawn`
(approver). Platform/version claims are not currently part of the enrollment
intent and must not be fabricated by the Console.

Signed `EnrollmentIntent` always contains `existing`: explicit `null` for initial
enrollment, or the full existing binding for reauthentication. Include it in the
canonical intent digest; omission is rejected by both generated validation and
Rust decoding. Closing an unapproved browser flow does not cancel the device's
request. `denyDeviceAuthorization` withdraws only the caller's prior approval
before redemption; the device can use signed `cancelEnrollmentRequest` itself.

Enrollment-key issuance validates the initial hostname/listeners and any supplied
TLS materials. A 422 `configuration_invalid` response carries issue paths rooted
at `/initialSettings`. Missing automatic materials are generated at actual Node
admission; issuing a key does not create a Node or reserve its hostname.

`listEnrollmentKeys(client, { networkId }, { limit, cursor }, { token })` now has
an explicit query argument; pass `{}` for defaults. It and `listJoinLinks` return
descending, creation-bounded pages with `total`, `nextCursor` and `asOf`. Each
page rechecks the current identity. Key views include creator, consumption time,
Node ID and non-secret initial-settings summaries; secret delivery remains
one-time. Use `NetworkActions.joinLinkRoles` for role choices, and link/key
`actions.revoke` for their current revocation eligibility.

Node inventory separates `canDelete`, `canRevokeControlKey` and
`canRevokeDataplaneKey`, includes key creation times and `desiredExecution`.
That reference describes the current target, not an accepted receipt.
`getNodeSettings(client, { networkId, nodeId }, { token })` returns
`{ requestId, networkId, nodeId, revision, settings }`; the non-secret settings
fragment is shared with SSE and bound to this exact revision. Ordinary members
can read only their current User-identity devices. Creation history grants no
read permission after a device switches to Tag identity; full configuration
history remains restricted separately.

`replaceDefaults` accepts omitted `access`, `topology` and `dialers` as keep.
Submit only explicitly edited fields when revalidating against a new revision;
the service still requires strict CAS. A config request ID reused with different
input returns `request_id_conflict`; an old base returns `revision_conflict`.
Key generation and invalid device transitions use `version_conflict` and
`state_conflict` respectively. Join-link inspection distinguishes used, revoked,
expired and unavailable-issuer errors, and supplies `alreadyMember` for a valid
link. Activity/revision user actors include required nullable `email`, captured
at event time; never fill a missing snapshot from the user's current address.

HTTP time fields use RFC 3339 (`expiresAt`, `registeredAt`, `reportedAt`, etc.).
Rust execution/crypto structs retain explicit `_ms` fields with a validated
Serde adapter; fixed signing transcripts still encode exact Unix milliseconds.
No unbounded integer timestamps are exposed to TypeScript.

The live SDK test uses Node's Ed25519 implementation and generated DTOs to run
normal browser approval, signed PKCE redemption, a Node session, lease/config
reads, authenticated State reporting and delta delivery, release and permanent
DELETE against the Controller binary. It does not install a tunnel or manufacture
an applied/data-plane result.

## Event streams

The generator supports GET SSE operations explicitly declared by the actual
route's `x-meshway-sse` metadata. A successful `text/event-stream` response is
string bytes; the extension's named event schema describes each JSON `data`
payload. JSON errors keep their ordinary typed HTTP envelopes.

- TypeScript generated stream methods return an async generator of
  `ClientResult<EventDelivery<Event>, Problem>`. Every event is checked with its
  generated Zod schema. `AuthenticatedStreamOptions` requires a token;
  `signal` cancels pending connection/read/retry work.
- TypeScript retries connection loss, EOF, 429 and 5xx up to `maxReconnects`
  (default 5). `resumeCursor()` is called for each connection and must return
  only a cursor whose complete snapshot/delta has been applied by the caller.
  Merely decoding an event or seeing an SSE `id` never advances that cursor.
- `idleTimeoutMs` bounds pending network I/O (default 30 seconds, configurable
  from 1 ms through 5 minutes). A timeout closes the connection and reports a
  `network` failure before the usual bounded retry. Actual bytes, including
  keepalive comments, refresh the deadline; empty chunks do not. Time spent by
  the caller applying a delivered event does not consume this I/O deadline.
- Rust generated stream methods take `StreamOptions` and return an
  `EventStream<Event, Problem>`. `next().await` yields one typed event, a terminal
  error, or `None` at EOF. Drop the stream to cancel. The caller reconnects with
  its own lease/retry policy and explicitly supplies `applied_cursor`; the
  bounded `retry_delay()` hint is available after frames have been read.
- Both runtimes bound UTF-8 lines and complete event payloads by
  `maxFrameBytes` / `max_frame_bytes` (1 byte–16 MiB, defaulting to the client body
  budget). EOF discards incomplete frames. Invalid UTF-8, JSON, event types and
  oversized frames terminate decoding without exposing raw content in errors.

The normal Controller Router provides `watchNetwork` and `watchNode` with the
root-exported `UserEvent` and `NodeEvent` contracts. `getNetworkState`,
`getNodeState`, `reportNodeState`, `queryPeerState`, `reportNodeHistory` and
`queryObservationHistory` use the same real identity and visibility rules.

```ts
const events = watchNetwork(client, { networkId }, {
  token, signal, resumeCursor: () => appliedCursor,
});
```

The initial sequence is `config.snapshot`, `entities.snapshot`, `state.snapshot`,
then `sync.ready`. Stage the complete batch before installing its cache and ready
cursor. Manager Config snapshots contain the full non-secret view; member
snapshots contain only their own permitted Node settings. Node snapshots contain
their own non-secret settings and the actual target reference. No source JSONC,
private key, password or hidden header is broadcast. Config changes invalidate
server-derived caches without overwriting the caller's draft or CAS base.

`getConfigurationProgress(client, { networkId, revision }, { token })` returns
`ConfigurationApplyProgress`, also used by initial snapshots and progress events.
The fixed target set belongs to that configuration revision; later demands and
refreshes do not enlarge its denominator. Summary phases represent original
targets' accepted receipts; per-node `freshness` separately identifies expired,
superseded or awaiting-instance evidence. Historical `applied` is not current
health. Full/partial scope and counts follow the requesting identity.

## Managed rule sets

Owner/Admin use `listRuleSets(client, { networkId }, { token })` and
`getRuleSet(client, { networkId, ruleSetKey }, { token })` to read a `RuleSetList`.
`refreshRuleSet` takes the same key path and returns a `RuleSetView`. Ordinary
members cannot read source definitions or refresh them.

A view separates the immutable `artifact` from `checkedAt`, `lastSuccessAt` and
`problem`. `publication` identifies the exact execution publication that selects
the content. Refreshing does not change the source configuration revision;
failed checks retain the last successful artifact. `resources.changed` on the
management SSE stream invalidates this read view without broadcasting contents
or source URLs. Publication/receipt evidence does not attest to actual traffic.

An `applyConfiguration` restore request may supply `resourcePublication` to use
that same source revision's historical resource bindings; omit it for the source
version's original bindings. Cross-network or mismatched source publications
are rejected. A single configuration apply can prepare a newly referenced remote
source or replace an existing one. Validation returns tentative digest/entry
counts in `preparedRuleSets`, without storing artifacts or allocating fake IDs.
The Controller starts periodic `updateInterval` checks with bounded retry;
unchanged canonical contents do not create a new execution publication.

## Active path observations

`createObservation` takes `sourceNodeId`, `targetNodeId`, a distinct per-page
`subscriptionId`, `service` (`{ type: "tcp" | "udp", port }` or
`{ type: "icmpEcho" }`), `selection` (`{ mode: "auto" }` or an explicit path),
`metrics` and `intervalMs`. The exact service and full path must be authorized.
Idle networks can prepare independent diagnostic permits without opening any
ordinary business flow. Source capability `feature:path-observation` version 1
is required; inspect `unsupportedMetrics`, including current `traffic` support.

`listObservations`/`getObservation` use subscriber views; `renewObservation` and
`cancelObservation` act on the subscription ID, not the reusable task ID. Renew
only after the returned `renewAfter`. Each tab has its own lease; closing one
subscription does not stop another. The last dependency expires or cancels
independently of Node heartbeats.

`getNodeTasks` returns actual tasks for the authenticated Node/instance, while
`reportTaskResult` uses the task ID plus its exact instance/configuration.
Result `phase` can report failure or unsupported without a made-up path/sample;
`sample` requires real measurement evidence. Task changes may produce a new
execution target: the Node follows its normal lease/configuration loop before
executing/reporting against that target. Results are forwarded through
`observation.status`/`observation.sample`; terminal subscriptions are removed
without replaying cancelled samples. These APIs do not prove an Agent has
installed or measured a tunnel.

The SDK's TypeScript check covers both browser libraries and a Node-only
`ES2022`/`@types/node` configuration; Node consumers do not need DOM types.

## Generation and verification

From the repository root, using this task's isolated Cargo target when working in
parallel:

```sh
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR="$PWD/target"
node scripts/generate-protocol.mjs
node scripts/check-protocol.mjs
node --test scripts/generate-protocol.test.mjs
cargo test -p meshway-control-client --locked
cargo build -p meshway-server --bin meshway-server --locked
pnpm --filter @meshway/control-client typecheck
MESHWAY_SERVER_BINARY="$CARGO_TARGET_DIR/debug/meshway-server" pnpm --filter @meshway/control-client test
```

The integration tests start a real Controller on loopback with private temporary
storage. They test registration/session/logout, disabled registration, membership
actions and totals, join/role changes, configuration validation/CAS/restore,
State reporting, SSE synchronization/replay, resource refresh/restore and scope,
typed errors and malformed real response envelopes. Generator-only synthetic
fixtures verify code-generation boundaries separately.

The current generator supports the published account/network/Node JSON operations,
validated scalar path/optional query parameters, and explicitly declared JSON-data
SSE streams. JSON mutations support POST and DELETE. Undeclared streams and unsupported schema/operation shapes fail
generation explicitly. Integer schemas must prove a JS-safe range through
explicit bounds or the supported `int32` format; unbounded `int64` is rejected.
