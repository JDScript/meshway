# Persistent Docker neighbors in the development Web console

This is **live-demo tooling**, separate from the isolated acceptance fixture in
`README.md`. Run it only after the coordinator has obtained the isolated managed
Docker PASS and chosen to seed the existing development controller. Implementing
or locally testing these scripts does not constitute that acceptance or authorize
contacting a controller.

The three registered names are **demo-nyc**, **demo-lax**, and **demo-sha**. The real
Agent CLI creates their identities, sends heartbeats, publishes connection keys
and capabilities, applies controller intent, and reports authenticated neighbor
nonce probes. Open the existing dev Web console connected to the same controller
to inspect these real node records and runtime observations.

## Commands

Requirements: non-root Unix host user, Node **24**, Python 3, host `openssl`, Docker,
installed workspace pnpm dependencies, and a **prebuilt** native image from
`experiments/managed-connections/Dockerfile` containing the managed Agent. Supply
the coordinator's retained image; the demo never builds, retags, pulls or deletes
images. The image ID is pinned on first initialization. Ordinary `start` continues
to reject a changed image; use the explicit image-only upgrade below.

From the workspace root, after the coordinator's isolated acceptance:

```sh
MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock \
  python3 experiments/managed-connections/demo.py start \
  --data-dir .meshway/docker-demo \
  --image meshway-managedtest:integration-20260930-a \
  --controller-url http://127.0.0.1:8420 \
  --agent-controller-url http://host.docker.internal:8420 \
  --admin-token-file .meshway/controller/admin.token

python3 experiments/managed-connections/demo.py status \
  --data-dir .meshway/docker-demo \
  --admin-token-file .meshway/controller/admin.token

python3 experiments/managed-connections/demo.py stop \
  --data-dir .meshway/docker-demo
```

Substitute the image tag actually built and retained by the coordinator. The data
parent must already exist at its canonical, non-symlink path. The selected data
directory is created 0700 or must be an **empty, current-user-owned 0700** directory
on first use. A populated directory without valid demo metadata is rejected. The
admin token file must be a current-user-owned, regular, non-symlink 0600 file.
The admin token is read into host SDK memory; it is never mounted into a container.

`--controller-url` is the **host SDK** URL. `--agent-controller-url` is the distinct,
explicit URL stored in each Agent's immutable identity. Both are normalized and
pinned in demo metadata; subsequent starts must supply the same values. URLs must
not contain credentials, query strings or fragments. The provided loopback/host
alias pair is intended for the previously verified OrbStack host connectivity.
Linux users must supply a controller address actually reachable from Docker. If
appropriate for their host, add `--host-gateway` on the first and every subsequent
start to install `host.docker.internal:host-gateway`. That mapping does not make a
Linux loopback-only service reachable automatically.

All commands use the same shared performance lock as the isolated fixture. The
Python launcher passes the locked file descriptor into Node, retaining ownership
through cleanup. Do not wrap `demo.py` in another performance-lock invocation.
`--help` is read-only and needs neither the lock nor a data directory.

## Manual shell checks

The updated image installs **`meshway-agent`**, the **`meshway`** symlink, and
Debian **`iputils-ping`**. It sets `MESHWAY_STATE=/state/agent.json`; explicit
`--state /private/...` arguments in the isolated fixture still take precedence.
Existing containers need an image upgrade/recreation before these are available.
Do not install packages in the running, read-only demo containers.

From the host, discover the actual container names by labels. This wrapper keeps
even manual Docker actions under the shared lock (exit an interactive shell to
release it):

```sh
export MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock
demo_docker() { python3 experiments/transport/performance_lock.py docker "$@"; }
demo_docker ps --filter label=io.meshway.demo.project=managed-neighbors \
  --format 'table {{.Names}}\t{{.Label "io.meshway.demo.resource"}}\t{{.Status}}'

# Substitute the exact NYC name printed above; the hash depends on --data-dir.
demo_docker exec -it meshway-demo-DATA_HASH-nyc sh
```

Inside NYC, after the coordinator has upgraded to the validated diagnostic image:

```sh
meshway status
meshway connections status
meshway ping demo-lax --count 3 --timeout-seconds 3
meshway ping demo-sha --count 1 --timeout-seconds 3
ping -c 3 127.0.0.1
ping -c 3 lax.mesh.test
```

- `meshway status` and `meshway connections status` read local saved metadata;
  they do not themselves prove reachability.
- **`meshway ping` sends a fresh authenticated direct-neighbor probe through the
  running managed Agent.** NYC → LAX uses native UDP. SHA → LAX uses VLESS/TLS.
  From **LAX**, both `meshway ping demo-nyc` and `meshway ping demo-sha` exercise
  the accepted, peer-initiated bidirectional channels. No extra port or admin
  secret is needed: the CLI uses the private `/state/agent.json.control.sock`.
- NYC → `demo-sha` **must fail**: SHA is not NYC's direct authenticated neighbor.
  The CLI does not automatically route through LAX. It is not ICMP or mesh trace.
- Standard **`ping lax.mesh.test` tests only the Docker underlay**, independent
  of Meshway authentication. Unprivileged Linux ping datagram sockets are enabled
  by `net.ipv4.ping_group_range=0 2147483647`; UID/GID remains non-root, capabilities
  remain fully dropped, and no `NET_RAW`, `NET_ADMIN` or privileged mode is added.
- **`ping 100.96...` is unsupported in this managed demo.** Allocated mesh addresses
  are control-plane metadata here; there is no TUN/business-IP forwarding. The
  real adjacency graph has two edges, NYC–LAX and SHA–LAX, not a full mesh.

These manual authenticated-CLI checks are **deferred to the coordinator** after
Agent integration and image validation; the shell smoke test below does not claim
they passed on the persistent demo.

## Explicit image-only upgrade and recovery

Build and validate a new, host-native image before touching the persistent demo:

```sh
MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock \
  python3 experiments/transport/performance_lock.py docker build \
  --file experiments/managed-connections/Dockerfile \
  --tag meshway-managedtest:NEW .

python3 experiments/managed-connections/shell-smoke.py --image meshway-managedtest:NEW

# Run only when the coordinator is ready to restart the three demo Agents.
python3 experiments/managed-connections/demo.py upgrade-image \
  --data-dir .meshway/docker-demo --image meshway-managedtest:NEW \
  --admin-token-file .meshway/controller/admin.token
```

`shell-smoke.py` acquires the shared lock itself and uses two uniquely labeled
scratch containers on one new internal bridge. It checks native architecture,
managed binaries, the alias/default state environment, and real `ping 127.0.0.1`
and `ping lax.mesh.test` under the demo's non-root/cap-drop/read-only restrictions.
It checks the authenticated ping CLI's help/flags if present, but has no enrolled
nodes and does not test authenticated probes. Scratch resources are removed with
ownership checks in `finally`; the supplied image is retained. No live directory,
controller or demo network is used.

`upgrade-image` is deliberately distinct from `start`:

1. Require ready metadata and all three original enrolled/published identities.
   Validate canonical private directories, pinned controller URLs/UID/GID,
   material hashes, certificate validity, Docker ownership labels, expected image
   IDs, original `/state` mounts and non-root/container security settings. Both
   original and target images must be Linux and match the host and Docker engine
   architecture; emulation is rejected. The target needs both managed binaries,
   the shell alias/default environment and ordinary ping. If it exposes the new
   `ping` CLI, its `--count` and `--timeout-seconds` flags are checked.
2. Bind the original node directory **read-only** at `/source-state` in network-free
   check containers. Copy only `agent.json`, `agent.json.connection-key.json`, and
   optional `agent.json.config.json` / `agent.json.connections.json` into a private
   0700 directory on writable `/state` tmpfs, with 0600 files. The actual Agent
   `status` / `connections status` commands validate these exact copies, including
   key-pair checks: their protocol reader requires read+write opens even for status.
   Original source opens are strictly read-only, bounded and no-follow; foreign
   owners, symlinks, hardlinks, non-regular files and wrong permissions are rejected.
   Original identity/key hashes are checked again afterward. Profiles, locks,
   sockets and unrelated files are never copied; temporary copies are discarded.
   Prove each original credential still authenticates its recorded node ID using
   the generated node get-connections operation. Check inventory/public-key
   bindings through the generated admin SDK. No enrollment helper is run in
   recovery/publish mode, and no grant is minted.
3. Before any stop, atomically save private `upgrade.json` with old/new image pins,
   the intended target and phase, immutable metadata/identity/key/profile hashes,
   and the original global connection-plan revision/hash. Gracefully stop only
   owned containers, then read the per-node controller runtime epochs through
   generated `getConnectionRuntimes` (zero only if absent, including stale/withdrawn
   epochs otherwise). Persist this `epoch_floor` **before any replacement starts**.
   Atomically change only `demo.json.image_id`, recreate the containers on their
   original networks/state mounts, and start the same Agents.
4. Require the normal fresh authenticated-neighbor graph and unchanged identity,
   key/profile bytes and global plan. **Every node's reported epoch must be
   strictly greater than its recorded floor**; even a fully healthy, recent report
   from the preceding process cannot complete verification. The tool never saves or repairs the plan.
   Agent epochs/reports, heartbeats and saved caches can advance normally. Mark
   the journal complete and durably remove it. Repeating an already-pinned image
   validates readiness and immutable state without restarting healthy containers.

An interrupted upgrade retains every original credential and private file. Retry
the exact same `upgrade-image --image ...` command to complete it. A running
replacement cohort and its durable epoch floor are retained on verification
failure. A retry rechecks this same cohort without restarting or recapturing its
already-advanced epochs. If a replacement is absent/stopped (including a crash
during a partial start), retry performs a new controlled restart and records a
new post-stop floor for that genuinely new cohort. Switching targets for rollback
also records its own floor before starting rollback replacements. `status` prints
public old/new/target image IDs, the pending phase and epoch floor. Ordinary `start` refuses a
pending journal, so it cannot accidentally bypass an incomplete migration.
For an explicit **pending-upgrade image-only rollback**:

```sh
python3 experiments/managed-connections/demo.py rollback-image \
  --data-dir .meshway/docker-demo --admin-token-file .meshway/controller/admin.token
```

Rollback uses the recorded old image ID and the same state; it never restores,
deletes, replaces or regenerates credentials or keys. Keep the old image available
until upgrade verification completes. The first shell upgrade can roll back to a
legacy image without the new alias/ping CLI. A changed plan, key/material, revoked
node, foreign image/resource, incompatible disk format or unavailable controller
causes a guarded failure; concurrent plan edits are never overwritten. Such a
failure requires inspection before recovery can pass. There is no automatic
credential/schema downgrade or controller migration.

The private upgrade journal is now **version 2**, adding
`epoch_floor: {nyc, lax, sha} | null`. `prepared` has a null floor; `restarting`,
`verifying` and `complete` require all three nonnegative integer floors. The
`verifying` phase is durable before the first replacement starts. Existing
version-1 upgrade journals are read compatibly with pins and immutable hashes
retained, but are treated as `prepared` and must undergo a new epoch-fenced
restart; an old unfenced `complete` phase cannot bypass verification.
`demo.json` remains version 1. No controller API or generated schema changes.

The separate `verify-smoke.py --image OLD_PIN --image NEW_PIN` regression uses
synthetic, never-enrolled identity/key files and the real binaries from both
images. Under the shared lock it proves the original bind rejects writable opens,
the private-copy checks succeed, original bytes/metadata remain unchanged and
copies/containers are cleaned up. It has `--network none`, no live mounts, and
does not access a controller. No image rebuild is needed for helper-only changes:
the upgrade mounts the current `demo.py` read-only from the host.

The diagnostic socket is checked as an optional **actual 0600, current-user Unix
socket**. A regular file or symlink at that path is rejected. It is not fingerprinted
as key material, and the harness never unlinks it; the Agent owns shutdown and
stale-socket handling under its state lock.

## Start, rerun and stop semantics

1. Initialize private per-node profiles once and pin their content hashes. Check
   the preexisting controller inventory and reserved IDs before enrolling nodes.
2. Verify exact Docker labels on every existing demo resource. Gracefully stop and
   recreate only those owned containers, using the same private bind mounts. This
   controlled restart on every `start` gives the Agent state lock one owner and a
   new real runtime epoch; it does not create another node identity.
3. Recover existing credentials through the real `heartbeat` CLI. For a pending
   first enrollment, retain the original grant in a private per-node file until
   confirmed success. Retry that original controller/name/token/credential binding
   if heartbeat cannot yet recover it. Delete the pending grant after the ID is
   durably recorded. An enrolled credential rejected by the controller never falls
   back to enrollment. A missing/changed previously bound Noise key is rejected.
4. Publish compiled capabilities using the real CLI, then use generated SDK
   `getConnections` / `saveConnections` with global revision CAS. Retry conflicts
   against a newly fetched plan; a repeated identical merge does not save again.
5. Start the three Agent containers with:

   ```text
   run --apply-connections --interval-seconds 2 --connection-lease-seconds 15
       --connection-profiles /state/profiles.json --state /state/agent.json
   ```

6. After stopping originals, capture the controller epochs before starting any
   replacement. Wait up to 90 seconds for all three current, non-stale, applied runtime reports,
   matching saved revision/digest receipts, positive epoch/sequence, and receive
   age at most eight seconds. Each epoch must be **strictly newer than that node's
   post-stop floor**, including an ordinary controlled `start` on the same image.
   Require both sides of each expected adjacency to
   report authenticated, `probe_fresh=true`, non-null measured RTT (zero is valid),
   and the exact peer/listener/candidate/initiator association. SHA must initiate
   VLESS on **both** reports. Saved ACK alone is insufficient.

`stop` needs no admin token or reachable controller. It sends the configured
SIGINT and waits up to 20 seconds per owned container. It preserves registered
identities, connection keys, profiles, files, Docker networks, stopped containers
and controller plan intent. It never revokes nodes or reenrolls. Agents normally
report shutdown withdrawal; if the controller is unavailable, observations age
stale. Docker running/stopped state is displayed separately from report freshness.

`status` prints only public controller URLs, names/IDs, local enrollment/container
phase, registration/contact metadata, and runtime epochs/states/probe observations.
It does not read/display profile bodies, credentials, UUIDs, PEM or raw logs. On
controller failure it prints unavailable observations, not synthetic health.

There is no destroy/revoke command. Networks and private state are intentionally
retained. No global Docker prune is used. Names contain a data-directory hash;
project, random run identifier, full directory fingerprint and resource labels
must all match before an existing resource is reused, stopped or removed.

## Underlay, plan ownership and private material

```text
demo-nyc -- internal data bridge A -- demo-lax -- internal data bridge B -- demo-sha
     \________________ additional management bridge ___________________/
                                     |
                         host development controller
```

LAX has alias `lax.mesh.test` on both data bridges. Candidates and advertised
endpoints use that DNS name, never allocated Docker IPs. All containers join the
separate management bridge for the explicit host controller route. No host Agent
listener ports are published. This persistent demo **is not a physical no-bypass
fixture**: management reachability exists between nodes. The isolated acceptance
fixture, whose controller resides in LAX and has no shared management network,
provides that topology test. Pinned Noise identities and listener-scoped grants
still authorize only the configured adjacencies in this demo.

Only these reserved entries are merged:

| Entry | Owner/direction | Transport |
| --- | --- | --- |
| `demo-lax-udp` | LAX, `0.0.0.0:41000` | native UDP + Noise |
| `demo-lax-vless` | LAX, `0.0.0.0:41001` | TLS + VLESS + Noise |
| `demo-nyc-to-lax` | NYC → LAX UDP listener | native UDP |
| `demo-sha-to-lax` | SHA → LAX VLESS listener | VLESS |

Unrelated entries retain their complete values and order. Reserved-ID collisions
with other owners or different candidate listener bindings are rejected. Matching
names do not authorize adoption of an existing node; full persisted node IDs and
original credentials are required. The existing local `nyc` node is not modified.
Extra saved listeners and candidates involving these demo IDs are preserved.
Readiness requires **all listeners declared for each node in the current saved
plan** to report bound, including user-added NYC/SHA listeners. Every reported
peer must match a saved listener, transport, dialer/acceptor role and candidate;
unknown or duplicate claims are rejected. Additional standby candidates need not
all authenticate, but both original NYC–LAX and SHA–LAX adjacencies must still
provide fresh authenticated probes in both directions. Upgrade snapshots the
current plan, so prior legitimate Web edits are retained rather than reverted.

Each process runs as the host's non-root UID/GID and receives only its own 0700
node directory at `/state`; config, pending grants and material files are 0600.
The common enrollment helper source is mounted read-only into temporary helper
containers. No node sees another node's state, or the admin token. Metadata uses
private atomic writes with file/directory fsync. Private profiles use the frozen
version-1 schema in `apps/agent/MANAGED_CONNECTIONS.md`:

- NYC: an empty profile list.
- LAX: server-only `demo-lax-server`, stable VLESS UUID, leaf certificate and key.
- SHA: client-only `demo-lax-client`, same wire UUID, CA certificate and verified
  server name `lax.mesh.test`.

Host OpenSSL generates a temporary private CA directory, signs a **30-day** leaf
with SAN `DNS:lax.mesh.test`, installs the scoped material, and removes the issuer
directory. That directory is never mounted. Reruns reuse all material and refuse
changed/missing files or near-expired certificates. There is no automatic rotation.
No bearer, UUID or PEM is passed in host Docker argv. Enrollment input goes through
stdin to a container-local helper; the current Agent CLI's token argument exists
only inside that private container. Child output and exceptions are suppressed;
Docker log collection is disabled. No public artifact contains private material.

Containers drop **all** capabilities, use a read-only root filesystem, disable IP
forwarding, and need neither TUN, NET_ADMIN nor NET_RAW. The channels carry only
authenticated neighbor control probes, not business-IP traffic, transit or exits.

## Partial failures and recovery limits

- Failed starts stop owned containers where possible, then print public status.
  Files, identities, networks, and any already-saved plan intent remain inspectable.
  Any resource that cannot be stopped is explicitly named by role. SIGINT/SIGTERM
  follow the same cleanup after the bounded in-flight command completes.
- Image upgrades retain their journal and running replacement cohort on verification
  failure so retry can use the original epoch fence. An explicit `stop` remains
  available; a later upgrade retry must then record a new floor for its new cohort.
- A lost enrollment response reuses the original grant/credential; a committed
  enrollment can recover by heartbeat after grant expiry. An expired **unused**
  grant is not silently replaced.
- Lost grant-creation responses leave `grant_requested`; interrupted initial
  certificate creation leaves `preparing`. Missing original Agent state after
  `enroll_started`, missing bound keys, or unfamiliar files require manual private
  inspection. The tool deliberately refuses automatic reset, regeneration or
  name-based adoption. Do not delete state and rerun against existing registrations.
- Controller database replacement, revocation, changed URLs/UID/GID, moving the
  private directory and certificate rotation require deliberate migration outside
  this tool. Only `upgrade-image` / pending `rollback-image` can change the image
  pin. Ordinary start/stop is not migration.
- Docker commands are bounded: ordinarily 30 seconds, enrollment helper 75 seconds,
  key publication helper 45 seconds. Agent CLI children have 30-second deadlines;
  generated SDK calls have their existing 10-second HTTP deadline. No build,
  performance workload, or acceptance outage/revocation phase runs here.

## Offline verification

```sh
pnpm exec tsc --noEmit -p experiments/managed-connections/tsconfig.json
node --test experiments/managed-connections/demo.test.ts experiments/managed-connections/upgrade.test.ts
python3 experiments/managed-connections/demo.test.py
python3 experiments/managed-connections/demo.py --help
```

Tests cover non-destructive/idempotent merges, ownership collisions, stale or
incomplete runtime evidence, SHA initiation, pending/enrolled credential/key
recovery guards, image-only idempotence, interruption/retry at every journal
boundary, image-only rollback, material/plan drift, foreign images/state mounts,
native architecture and actual private socket handling. Epoch regressions cover
healthy old reports, partially advanced cohorts, crash/retry without moving a live
cohort's floor, dead-cohort replacement, rollback floors and V1 journal recovery.
They use no real Docker
resources or controller and are not a claim of live-demo upgrade success.
Private-copy unit regressions also verify source read-only opens, unchanged
original bytes/metadata, writable disposable copies, no IPC/profile copying,
cleanup after CLI failure, and link/special-file/permission rejection. The Docker
copy smoke above separately checks the real old/new CLIs against read-only binds.
