# Controller-managed Docker adjacency acceptance

This fixture creates a **new isolated controller and three really enrolled Agents**.
It exercises generated controller configuration → Agent application → authenticated
neighbor nonce probes. The older static transport-lab fixture is not used.

**Implementation status:** the complete Docker acceptance passed with run
`muoz8o7j-613526f5`, using image
`sha256:82cec63170602b8b75dad7a0a0a4815286e48d9614971be85a78f92ba03af074`.
`result.json` records every required phase and successful cleanup. This is a
functional adjacency/probe result, not business-IP routing or a performance result.

## Run

From the workspace root, with Node **24**, Python 3, host `openssl`, the repository's
installed pnpm dependencies, and a native Docker engine:

```sh
pnpm exec tsc --noEmit -p experiments/managed-connections/tsconfig.json
node --test experiments/managed-connections/checks.test.ts

MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock \
  python3 experiments/managed-connections/run.py
```

The final command is intended for the coordinator's tmux session. It acquires the
existing shared `performance_lock()` **before any Docker command**, inherits the
same lock into Node, builds a unique `meshway-managedtest:<run-id>` image, runs the
functional phases, and removes its own containers/networks/image. No performance
load is generated. Host, engine, and image architecture must match.

The Dockerfile uses the same pinned Rust **1.98** and Debian trixie base digests as
`apps/agent/Dockerfile.lab`, but installs **both** controller and Agent binaries and
enables `meshway-agent/managed-connections` (which includes `transport-lab`).
It also provides standard `iputils-ping`, the `meshway` alias for `meshway-agent`,
and `MESHWAY_STATE=/state/agent.json` for the persistent demo's interactive shell.
The isolated fixture's explicit `--state /private/...` paths override that default.
`DEMO.md` distinguishes standard Docker-underlay ICMP from the new authenticated
direct-neighbor `meshway ping` CLI, and documents both the guarded image-only
upgrade and explicit writable Ubuntu developer image (`Dockerfile.dev`). The
isolated acceptance fixture and ordinary persistent mode remain read-only.

To build a reusable image with visible build diagnostics, run separately under the
same lock, then give the runner its tag. Choose a unique tag:

```sh
MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock \
  python3 scripts/performance_lock.py docker build \
  --file experiments/managed-connections/Dockerfile \
  --tag meshway-managedtest:integration-20260930-a .

MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock \
  python3 experiments/managed-connections/run.py \
  --image meshway-managedtest:integration-20260930-a
```

An explicitly supplied image is retained. The runner rejects other image
repositories. Options:

- `--artifacts-dir EXISTING_PARENT`: an existing directory outside the workspace;
  a new private child is created. Default: the approved OpenCode temporary directory
  shown in the lock path above.
- `--lease-seconds 10..45`: default **10** for this finite functional test. The
  production Agent default remains 45; poll interval here is **1 second**.
- `--help`: prints usage without touching Docker.

Do not wrap `run.py` in a second performance-lock invocation; it acquires that lock
itself. Direct invocation of `run.ts` is rejected without the inherited lock FD.

## Topology and evidence

```text
NYC -- internal net A -- LAX -- internal net B -- SHA
                       │
                controller :8420
```

`lax.mesh.test` is a Docker DNS alias on **both** networks. Every Agent, including
LAX, enrolls against `http://lax.mesh.test:8420`. The controller API alone is
published on an ephemeral **127.0.0.1** host port for the generated-SDK orchestrator.
Only LAX additionally joins a non-internal controller-API bridge: OrbStack/Docker
does not publish host ports for an internal-only container. NYC and SHA do not
join that bridge, LAX forwarding stays disabled, and it supplies no endpoint
bypass. The two data bridges remain internal.

All three containers have `--cap-drop ALL`, no TUN device, read-only image files,
0700 private tmpfs state, and IPv4/IPv6 forwarding disabled. Independent TCP canaries
prove that NYC and SHA cannot directly reach each other while both reach LAX.
No explicit subnet reservation, host route edit, NAT traversal, or punching is used.
The canary ports are private underlay-only diagnostic listeners, not Meshway channels.

The runner then:

1. Gets the isolated controller's private admin token with captured output; creates
   three one-time enrollment grants using the generated `createEnrollmentToken`.
   The **real Agent enrollment CLI** generates/persists each identity and node bearer.
   All registered IDs must be distinct full 64-character lowercase hex IDs.
2. Starts real `run --apply-connections` loops and waits for published public keys
   and the matching compiled capabilities. Checks the initial routing policy is empty
   with default reject. It does not modify routing policy.
3. Reads `getConnections`, then CAS-saves a plan using generated `saveConnections`:
   - LAX native UDP listener `0.0.0.0:41000`, candidate **NYC → LAX**;
   - LAX VLESS/TLS listener `0.0.0.0:41001`, candidate **SHA → LAX**;
   - both candidate/advertised hosts are **`lax.mesh.test`**.
4. Uses generated node `getAgentConnectionsOperation` to obtain the exact
   node-scoped digest/revision. Generated admin `getConnectionRuntimes` must show
   all three current, non-stale, applied, positive-epoch/sequence observations, no
   older than six seconds. The separate saved ACK digest/revision must also match.
5. Demands **both sides** of each adjacency report authenticated, `probe_fresh=true`,
   measured non-null RTT (including valid zero), the correct full peer/initiator IDs,
   carrier, listener, and candidate association. It separately inspects the actual
   SHA-initiated TCP connection and LAX accepted socket, and absence of a SHA VLESS
   listener. A TCP socket alone cannot satisfy the health predicates.
6. Removes SHA's local VLESS profile without changing identity or controller plan:
   SHA must report `error/profile_unavailable`, empty resources, and close the
   outgoing socket. Restores the profile and demands the authenticated graph again.
7. CAS-removes the SHA candidate: SHA must apply a **valid empty plan** with no
   neighbors; LAX must have no SHA incoming approval or observation. Restores the
   candidate and demands real recovery.
8. Stops only the controller process while Agents and dataplane networks remain
   running. After the finite lease expires, requires all managed listeners and
   established VLESS sockets to close, and **zero Agent-owned UDP sockets**, including
   ephemeral dialer sockets. Agents must still be running. The offline evidence is
   local socket/process inspection, not invented controller reports.
9. Restarts the same controller with the same private SQLite state. Requires the
   fresh graph again, unchanged Agent IDs and runtime epochs, and increased report
   sequences. Runtime reports must not bump configuration revision.
10. Revokes SHA through generated `revokeNode`, requires generated node configuration
    retrieval to return **403**, SHA process/socket shutdown, and stale withdrawn
    admin metadata with no resources or receive timestamp.

The harness never calls runtime `begin`/`report`; those reports come only from the
real Agents. The controller validates their scope, but cannot independently attest
the truth of Agent observations. Epoch, revision/digest, `current`, server receive
age and probe fields are checked together; the artifact must not be presented as
cryptographic controller proof of an offline node's health.

## Private material and artifacts

Profiles follow the frozen schema in `apps/agent/MANAGED_CONNECTIONS.md`:

- Version 1 with a `profiles` array; IDs `lax-vless-server` and
  `lax-vless-client` are used as `credential_ref`.
- LAX's **server-only** profile contains `uuid`, `certificate`, and `private_key`.
- SHA's **client-only** profile contains `uuid`, `ca_certificate`, and
  `server_name: "lax.mesh.test"`.
- NYC receives an empty profile array. Certificate paths are relative to the
  private profile file; installed material is mode 0600.

A fresh two-day test CA signs a server certificate with SAN `DNS:lax.mesh.test`.
Only LAX gets the endpoint private key. SHA receives the CA **certificate**, never
the CA private key. The temporary host CA/key directory is removed immediately
after installation and in failure cleanup. Node credentials/connection private
keys are generated inside their own node container, not collected onto LAX.

Host Docker command arguments never include admin/grant/node tokens, UUIDs, PEM,
or profiles. Private transfers use stdin; subprocess failure details and captured
output are suppressed. The current Agent enrollment CLI only accepts `--token`,
so the container-local helper passes the enrollment token as the child Agent's
argument **inside that private container**, without printing it or including it in
the host command. The host SDK holds admin/node bearers in process memory only.

`result.json` is mode 0600 in a 0700 directory outside git. It contains public
node IDs, configuration/ACK snapshots, generated validated runtime observations,
socket/topology observations, completed phases and cleanup outcomes. It never
stores tokens, profile bodies, UUIDs, private PEM, raw Agent logs, or process argv.
On functional failure it attempts a final public API/socket snapshot. Reports are
not synthesized when the controller is unreachable. The artifact is retained even
when containers are removed; private container logs disappear with the fixture.

Docker operations have bounded timeouts (build: 20 minutes; ordinary command:
30 seconds; container helper: 40 seconds). Functional waits are bounded, normally
70 seconds per recovery. Ctrl-C/SIGTERM marks the run interrupted and enters the
same final cleanup after the in-flight bounded command. Cleanup checks each unique
resource's run label before removal; there is no prune or unrelated-state cleanup.

## Integration dependencies and limits

Required root work, owned by the coordinator/other implementation owners:

- Buildable Agent `managed-connections` feature and frozen CLI/profile behavior.
- Controller runtime routes/SQLite-v4 migration per
  `crates/protocol/CONNECTION_RUNTIME.md`.
- Regenerated `packages/control-client` DTOs, schemas and operations, including
  public `getConnectionRuntimes`; generated node get-connections operation.
- Synchronized `Cargo.lock` for `cargo build --locked`.

No root manifest changes or extra npm dependency are needed by this directory.
The local six-test suite guards acceptance semantics and subprocess-output privacy;
it is not a Docker substitute and never posts its synthetic predicate inputs.

This slice establishes **authenticated neighbor control probes only**. It does
not establish business-IP routing, TUN traffic, transit ACL authorization, or a
NYC-to-SHA application path. Negative wrong-CA/server-name/Noise-key and
different-listener-same-carrier authorization cases belong to the core/Agent test
suite; this harness currently exercises missing-profile, withdrawal, outage,
restart and revocation negatives.

The separate persistent demo is documented in `DEMO.md`. It retains identities in
a user-selected private directory and initially CAS-merges only its reserved plan
IDs into existing global configuration. Subsequent fully enrolled starts preserve
the complete current plan. Its explicit `enable-dataplane` / `disable-dataplane`
commands add a journaled, opt-in Linux IPv4 TUN lifecycle while preserving IDs,
credentials, profiles and controller intent. `tun-bootstrap-smoke.py` proves the
non-root NET_ADMIN-only bootstrap in an isolated, locked scratch namespace.
Its shared management bridge makes it inspectable from the live console; it does
not replace the isolated acceptance topologies.
