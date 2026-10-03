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
- `@meshway/control-client/schemas` exports `z<TypeName>` validators.
  `@meshway/control-client/json-schema` exports editor schemas.
- All bodies are validated. An invalid success body yields `decode`; an invalid
  error body yields `http` with its status and no `problem` or raw body.
- Other failure kinds are `configuration`, `request`, `network`, `body_limit`
  and `cancelled`. A network read failure retains the status when available.
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

These operations manage network metadata and initial configuration. They do not
claim that a Node has enrolled, installed configuration, or established a tunnel.

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
- Rust generated stream methods take `StreamOptions` and return an
  `EventStream<Event, Problem>`. `next().await` yields one typed event, a terminal
  error, or `None` at EOF. Drop the stream to cancel. The caller reconnects with
  its own lease/retry policy and explicitly supplies `applied_cursor`; the
  bounded `retry_delay()` hint is available after frames have been read.
- Both runtimes bound UTF-8 lines and complete event payloads by
  `maxFrameBytes` / `max_frame_bytes` (1–16 MiB, defaulting to the client body
  budget). EOF discards incomplete frames. Invalid UTF-8, JSON, event types and
  oversized frames terminate decoding without exposing raw content in errors.

These are generator/runtime capabilities. The currently exported account/network
slice has no production SSE operation yet; State service candidates still need
their real identity, configuration and execution providers wired into the Router.

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
cargo build -p meshway-controller --bin meshway-controller --locked
pnpm --filter @meshway/control-client typecheck
MESHWAY_CONTROLLER_BINARY="$CARGO_TARGET_DIR/debug/meshway-controller" pnpm --filter @meshway/control-client test
```

The integration tests start a real Controller on loopback with private temporary
storage. They test ordinary registration/session/logout, disabled registration,
typed errors and malformed real response envelopes. Generator-only synthetic
fixtures verify code-generation boundaries separately.

The current generator supports the published account/network JSON operations,
validated scalar path/optional query parameters, and explicitly declared JSON-data
SSE streams. Undeclared streams and unsupported schema/operation shapes fail
generation explicitly. Integer schemas must prove a JS-safe range through
explicit bounds or the supported `int32` format; unbounded `int64` is rejected.
