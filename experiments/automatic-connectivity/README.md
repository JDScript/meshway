# Automatic adjacency acceptance

This fixture runs normal Controller and Agent binaries with isolated identities,
one isolated Docker network, real TUN devices, and bidirectional HTTP packets.
Only the relay has platform-managed listeners. Endpoint nodes publish no ports
and have no listeners or hand-authored connection candidates.

The source policy grants only the two endpoints access to each other. The
Controller must derive their two relay adjacencies, deliver coherent execution
snapshots, and keep the same logical paths and revision during UDP → TCP → UDP
carrier failover and recovery. Each stage requires current authenticated probe
evidence and successful HTTP over TUN; saved ACKs alone cannot pass.
Grant withdrawal must remove generated adjacency and packet paths, deny both
directions, and recover after normal policy restoration.

The final stage keeps only the A→B grant: generated relay adjacency must still
carry the real HTTP response on the established reverse path, while a separately
initiated B→A HTTP connection is denied. Source candidates remain empty.

A bounded transparent UDP gate on the Controller fixture container drops UDP
in both directions for the failure stage. It forwards opaque carrier datagrams
to the relay during healthy stages. This exercises carrier loss and priority
recovery; it does **not** test NAT hole punching or prove a direct carrier.

```sh
python3 experiments/automatic-connectivity/run.py \
  --runtime <coordinator-prepared-runtime-image> \
  --cargo-cache <existing-private-cargo-cache>
```

The full workspace source manifest is snapshotted before compilation. Rust
builds with two jobs in a non-root, capability-free, bounded ordinary container.
The runtime fixture currently uses root processes with only `NET_ADMIN` for
Agent containers; it does not claim non-root Agent acceptance. Runtime images
must supply the audited namespace prerequisites, Python, iproute2, nftables,
iptables, and the usual system CA certificates.

To rerun a fixture change without rebuilding identical Rust inputs:

```sh
python3 experiments/automatic-connectivity/run.py --image <prior-fixture-image>
```

Every runtime container verifies the complete source manifest and binary hashes.
All Docker operations hold the shared performance lock and inherit its FD.
Owned containers and the network are label-checked and removed in `finally`.
Private source/build snapshots, image, command receipts, first failures and the
final `result.json` remain under TMP `meshway-automatic-connectivity-runs/`.
