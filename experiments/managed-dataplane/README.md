# Isolated managed IPv4 TUN acceptance

This fixture enrolls **three real Agents with a fresh controller**, saves routing,
connections, tags and ordered endpoint-access rules through the generated SDK, and exercises
Linux kernel ICMP and HTTP over controller-assigned mesh addresses. The main session
owns the image build and the long Docker run. A completed `result.json` with
`status: "PASS"` is the acceptance result; host checks alone do not certify delivery.

## Run from the workspace root

Prerequisites: Node 24, installed pnpm dependencies, Python 3, host OpenSSL, native
Linux Docker, `/dev/net/tun`, and the freshly regenerated SDK containing
`getMeshAccessOperation`, `saveMeshAccessOperation`, tag operations,
`previewFlowOperation`, and `getAgentDataplaneOperation`.
The image must contain the new `run --apply-dataplane` implementation, the controller,
iproute2, iputils-ping, Python 3 and the existing managed-connections container helper.
Python's standard HTTP client supplies the real TCP/HTTP body test; curl is optional.

Host checks (no Docker):

```sh
pnpm exec tsc -p experiments/managed-dataplane/tsconfig.json
node --test experiments/managed-dataplane/checks.test.ts
python3 -B experiments/managed-dataplane/container.test.py
```

Main builds a unique reusable image under the same shared lock:

```sh
MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock \
  python3 experiments/transport/performance_lock.py docker build \
  --file experiments/managed-connections/Dockerfile \
  --tag meshway-managedtest:dataplane-20261001-a .
```

Main's externally managed tmux pane then runs:

```sh
MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock \
  python3 experiments/managed-dataplane/run.py \
  --image meshway-managedtest:dataplane-20261001-a
```

`run.py` acquires the lock itself; do not nest it inside another lock invocation.
It forwards termination to Node and holds the inherited lock through cleanup.
The runner uses the immutable image ID resolved from the supplied dedicated tag.
It never builds or removes that image. `--artifacts-dir EXISTING_PARENT` selects an
existing parent outside the workspace for a new private artifact directory.
The default is the approved OpenCode temporary directory shown above.

For a short first-stage diagnostic under the same lock, append `--preflight-only`.
It creates and validates the exact isolated containers/networks, capability set,
private helper installation, underlay isolation and kernel settings, then cleans up
before creating certificates or starting the controller/Agents. Its distinct
`PREFLIGHT_PASS` result certifies Docker setup only; full acceptance still requires
`PASS` from a run without this flag. Public `check` markers identify each external
operation/security validation, with only an allowlisted error category on failure.

## Isolation

```text
NYC -- internal bridge A -- LAX -- internal bridge B -- SHA
                            |
                 fixed TCP controller proxy
                            |
                   API-only bridge -- controller
                                         |
                              127.0.0.1:ephemeral (host SDK)
```

Only LAX joins both data bridges. NYC and SHA do not join the controller bridge.
The controller runs in a separate container so stopping the LAX **Agent process**
does not break enrollment/configuration access. LAX's proxy has one fixed upstream
(`controller.mesh.test:8420`); it cannot proxy mesh or arbitrary underlay targets.
Independent underlay canaries prove the two endpoints cannot directly reach each
other, while each can reach LAX. Kernel IPv4/IPv6 forwarding stays disabled.

All containers use run-unique names plus ownership labels, read-only image files,
root-owned per-container `/private` tmpfs, and no host bind mounts. Agents run as
UID 0 solely inside these isolated namespaces: capabilities are dropped except
`NET_ADMIN`, with the TUN device explicitly passed. No `--privileged`, host network,
or `NET_RAW` is used. `ping_group_range` enables ICMP datagram sockets; successful
loopback ping proves the test tool is functional before any denial is accepted.
Agent containers set `net.ipv6.conf.default.disable_ipv6=1` at creation and verify
the inherited kernel setting, allowing IPv4-only TUN setup with read-only procfs.
Docker's optional `CAP_` inspect prefix is normalized before the exact capability
allowlist check; the effective kernel mask must still be NET_ADMIN only.
The controller has no added capability and is the only host-published container.
No existing state directory or live controller input is accepted.

Fresh TLS profiles verify SAN `lax.mesh.test` against an ephemeral CA. Only LAX gets
the endpoint private key; SHA gets the public CA certificate. Host certificate
scratch is removed immediately after installation. Admin and node bearers are held
in host memory and container-private files, never command diagnostics or artifacts.
The controller bootstrap token is read only from the isolated controller by the
host orchestrator; no Agent gets the controller's private directory.

## Acceptance sequence

Fixed Agent options: `--apply-dataplane --tun-name meshway0 --tun-mtu 1000`,
`--interval-seconds 2 --connection-lease-seconds 15`, state `/private/agent.json`.
`--apply-dataplane` implies managed adjacencies. The fixture checks full 64-hex node
IDs, controller-published public keys against the Agent's public status, and fresh
controller-assigned `100.96.*` addresses. It generates no static lab configuration.

1. Bind both listeners only at LAX. NYC dials native UDP; **SHA initiates VLESS/TLS
   to LAX**. Require authenticated nonce probes in both directions of each edge,
   correct initiators and identities, and independent TCP socket direction evidence.
2. Save default-reject routing with NYC → LAX → SHA and SHA → LAX → NYC. The forward
   selector combines exact source node/country with destination `/32` CIDR. The
   reverse selector uses source country and destination node. **Empty access denies
   ordinary OS ping and HTTP**, while the adjacency probes remain healthy.
3. Allow only the NYC endpoint to the SHA endpoint. The controller compiles the
   routed NYC → LAX → SHA path into the schema-v1 Agent grant format. Round trips
   remain denied; SHA's Linux `Icmp.InEchos`
   counter must increase, showing actual one-way delivery without raw capture tools.
   Change that source rule to one bidirectional rule, then require ping and distinctive
   HTTP bodies in both directions. Each HTTP listener binds only its assigned mesh address.
4. Create `tag:client` and `tag:server`, assign them to NYC and SHA through generated
   admin operations, and replace the exact-node rule with one bidirectional tag rule.
   Generated flow preview and real traffic must both retain NYC → LAX → SHA.
5. Prove endpoint ACL first-match behavior in both directions: an earlier wildcard
   reject to `tag:server` blocks forward delivery, while an earlier SHA/server →
   NYC/client reject blocks replies despite a later bidirectional allow. Removing only
   each reject recovers traffic; no stateful/conntrack authorization is assumed.
6. Remove and move SHA's server tag and require withdrawal, then restore membership
   and traffic. Update an assigned tag description and add `tag:west`; both assignments
   must remain intact. A default-allow empty ACL still follows mandatory via-LAX routing,
   and an explicit routing reject still denies it. Restore default-reject tag access.
7. Prepend an owned routing reject before the route; deny with endpoint access intact.
   Remove only that rule and recover. Change routing to default-direct: endpoint
   authorization still allows the pair, but delivery fails because NYC–SHA has no
   direct authenticated adjacency. The previous via-LAX route cannot be retained
   after the route change. Restore the via policy and recover.
8. Change SHA's source country so the reverse route selector no longer matches; deny and
    recover. Withdraw all endpoint access rules with routes intact; deny and recover.
   Then restart NYC with a valid saved candidate whose unique DNS name does not yet
   exist. Require continued retries with the old owned unreachable route and no TUN
   or cached grants. Add the alias to LAX's existing data bridge and require fresh
   packet-generation evidence plus real traffic recovery without restarting NYC.
9. Stop only the LAX Agent; check failure within the 15-second lease window while
   the separate controller remains reachable. Restart and require a newer LAX
   runtime epoch plus fresh successful data traffic.
10. Stop the controller beyond the local lease. All Agent processes must stay alive,
   close managed sockets, and retain a TUN sink or explicit unreachable mesh guard.
   Repeat real ping/HTTP failures. Restart the same controller, require fresh higher
   report sequences in the same Agent epochs, unchanged identities and real recovery.
11. Revoke SHA last. Its tag assignment must disappear while policy-referenced tag
   definitions remain, and attempting to delete the referenced definition must return
   422. The generated node dataplane operation must return 403, SHA must exit, and the
   controller must expose no fresh revoked adjacency claims. Packet
   delivery remains denied and owned resources are cleaned in `finally`.

Policy negatives start only after **current configuration ACKs**, a new local
`managed_dataplane` generation proof, correct kernel interface/address/MTU/route,
and current authenticated-neighbor reports. The proof's **revision and full snapshot
digest must match that node's generated `getAgentDataplane` response**; a newly saved
cache cannot promote an older running generation. The proof parser only exports
public allowlisted fields and validates the digest as 64 lowercase hexadecimal
characters. A latest proof with missing/malformed bindings makes the node unready,
even if an earlier log entry was valid. Its latest withdrawal also supersedes earlier
readiness. New policy revisions still require a new ready event, retaining the
process/epoch checks instead of accepting old startup output.
The ACK remains a saved receipt, not a delivery claim. Successful traffic is retried
to a deadline; expected denial must hold repeatedly over a duration after readiness.

Every probe uses the assigned mesh IP directly and the normal OS routing table,
without an interface override, environment HTTP proxy, DNS exit, or underlay retry.
A failed ping with an underlay route or a missing guard **fails the acceptance**.
Evidence contains policy/access revisions, public source-rule counts, per-node compiled
path counts (null for a revoked credential), exact
IDs/keys/IPs, neighbor reports, sanitized Linux routes/interfaces, local readiness,
and observed ping/HTTP results. It is functional evidence, not a throughput result.
Raw source-spoof/injection coverage belongs to Agent unit tests, avoiding NET_RAW in
the Docker fixture.

## Cleanup and failure artifacts

`result.json` is mode 0600 inside a mode 0700 directory. It never includes raw logs,
private caches, enrollment tokens, node/admin bearers, UUIDs or keys other than public
identity keys. Failure messages contain only the public phase/check name. The runner
does not issue runtime `begin`/`report` operations; those originate from the Agents.

`finally` removes only containers and networks successfully created by this run,
after verifying the exact ownership label. It never prunes, searches by broad name,
removes externally supplied images, or edits host routes. Cleanup errors force FAIL
and retain the exact owned resource names for the coordinator. Uncatchable SIGKILL
or engine loss can leave those labeled resources; normal SIGINT/SIGTERM is forwarded
and cleaned up. No Docker acceptance PASS is claimed until main executes the image.
