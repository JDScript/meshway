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
  types and the five account operations: `getAuthOptions`, `register`, `login`,
  `getAuthSession`, `logout`.
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

The current generator supports the published account JSON operations. SSE,
path/query parameters and other unsupported schema/operation shapes fail
generation explicitly. Integer schemas must prove a JS-safe range through
explicit bounds or the supported `int32` format; unbounded `int64` is rejected.
