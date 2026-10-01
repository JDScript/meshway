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
images. The image ID is pinned on first initialization.

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

6. Wait up to 90 seconds for all three current, non-stale, applied runtime reports,
   matching saved revision/digest receipts, positive epoch/sequence, and receive
   age at most eight seconds. Require both sides of each expected adjacency to
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
Extra non-demo entries involving these demo IDs are preserved; they can prevent
the exact expected three-node graph from passing the readiness check.

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
- A lost enrollment response reuses the original grant/credential; a committed
  enrollment can recover by heartbeat after grant expiry. An expired **unused**
  grant is not silently replaced.
- Lost grant-creation responses leave `grant_requested`; interrupted initial
  certificate creation leaves `preparing`. Missing original Agent state after
  `enroll_started`, missing bound keys, or unfamiliar files require manual private
  inspection. The tool deliberately refuses automatic reset, regeneration or
  name-based adoption. Do not delete state and rerun against existing registrations.
- Controller database replacement, revocation, changed URLs/UID/GID, moving the
  private directory, certificate rotation and changing the pinned image require
  deliberate migration outside this tool. Ordinary start/stop is not migration.
- Docker commands are bounded: ordinarily 30 seconds, enrollment helper 75 seconds,
  key publication helper 45 seconds. Agent CLI children have 30-second deadlines;
  generated SDK calls have their existing 10-second HTTP deadline. No build,
  performance workload, or acceptance outage/revocation phase runs here.

## Offline verification

```sh
pnpm exec tsc --noEmit -p experiments/managed-connections/tsconfig.json
node --test experiments/managed-connections/demo.test.ts
python3 experiments/managed-connections/demo.test.py
python3 experiments/managed-connections/demo.py --help
```

Tests cover non-destructive/idempotent merges, ownership collisions, stale or
incomplete runtime evidence, SHA initiation, and pending/enrolled credential/key
recovery guards. They use no real Docker resources or controller and are not a
claim of live-demo success.
