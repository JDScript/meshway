import type { ConnectionRuntimesSnapshot, ConnectionsSnapshot, NodeSummary } from "@meshway/control-client"

// Design-lab sample network (development builds only, see routes/lab.tsx).
// It mirrors the demo topology so every link state appears at least once.

const id = (char: string) => char.repeat(64)
export const LAB_NODES = { NYC: id("a"), LAX: id("b"), SHA: id("c"), TYO: id("d"), HOST: id("e"), FRA: id("f") }

export function labFixtures(now: number): {
  nodes: NodeSummary[]
  connections: ConnectionsSnapshot
  runtimes: ConnectionRuntimesSnapshot
} {
  const { NYC, LAX, SHA, TYO, HOST, FRA } = LAB_NODES
  const iso = (ago: number) => new Date(now - ago).toISOString()
  const node = (nodeId: string, name: string, seenAgo: number, index: number, pending = false): NodeSummary => ({
    id: nodeId,
    name,
    acknowledged_revision: 42,
    configuration_revision: pending ? 43 : 42,
    agent_version: "0.4.0",
    country: null,
    last_seen: iso(seenAgo),
    mesh_ipv4: `100.96.0.${index}`,
    mesh_ipv6: `fd73:96a6:cf5e::${index}`,
    registered_at: iso(86_400_000),
    revoked_at: null,
  })
  const nodes = [
    node(NYC, "demo-nyc", 5_000, 2),
    node(LAX, "demo-lax", 4_000, 3),
    node(SHA, "demo-sha", 6_000, 4),
    node(TYO, "tokyo-edge", 30_000, 5, true),
    node(FRA, "fra-relay", 8_000, 6),
    node(HOST, "nyc", 3 * 3_600_000, 1),
  ]
  const listener = (listenerId: string, owner: string, transport: "native_udp" | "vless") => ({
    id: listenerId,
    node_id: owner,
    transport,
    bind: "0.0.0.0:41000",
    advertised_endpoints: [{ host: `${listenerId}.mesh.test`, port: 41000 }],
    credential_ref: transport === "vless" ? "srv" : null,
  })
  const candidate = (candidateId: string, dialer: string, listenerId: string) => ({
    id: candidateId,
    dialer_node_id: dialer,
    listener_id: listenerId,
    endpoint: { host: `${listenerId}.mesh.test`, port: 41000 },
    priority: 0,
    credential_ref: null,
  })
  const connections: ConnectionsSnapshot = {
    revision: 42,
    valid: true,
    diagnostics: [],
    nodes: nodes.map((entry) => ({
      node: entry,
      public_key: null,
      capabilities: [],
      acknowledged_at: null,
      acknowledged_digest: null,
      acknowledged_revision: 42,
    })),
    configuration: {
      listeners: [listener("lax-udp", LAX, "native_udp"), listener("lax-vless", LAX, "vless"), listener("fra-udp", FRA, "native_udp")],
      candidates: [
        candidate("nyc-to-lax", NYC, "lax-udp"),
        candidate("sha-to-lax", SHA, "lax-vless"),
        candidate("tyo-to-lax", TYO, "lax-udp"),
        candidate("host-to-lax", HOST, "lax-udp"),
        candidate("nyc-to-fra", NYC, "fra-udp"),
        candidate("sha-to-fra", SHA, "fra-udp"),
      ],
    },
  }
  const peer = (patch: Partial<ConnectionRuntimesSnapshot["nodes"][number]["peers"][number]>) => ({
    peer_node_id: LAX,
    transport: "native_udp" as const,
    initiator_node_id: NYC,
    listener_id: "lax-udp",
    candidate_id: null,
    state: "authenticated" as const,
    reason: "none" as const,
    probe_fresh: true,
    probe_rtt_ms: 0,
    ...patch,
  })
  const report = (nodeId: string, peers: ReturnType<typeof peer>[]): ConnectionRuntimesSnapshot["nodes"][number] => ({
    node_id: nodeId,
    epoch: 1,
    sequence: 10,
    revision: 42,
    digest: "a".repeat(64),
    state: "applied",
    reason: "none",
    listeners: [],
    peers,
    reported_at: iso(3_000),
    stale_after_seconds: 60,
    current: true,
    stale: false,
  })
  const runtimes: ConnectionRuntimesSnapshot = {
    nodes: [
      report(NYC, [
        peer({ candidate_id: "nyc-to-lax", probe_rtt_ms: 68 }),
        peer({ peer_node_id: FRA, listener_id: "fra-udp", candidate_id: "nyc-to-fra", probe_rtt_ms: 81 }),
      ]),
      report(LAX, [
        peer({ peer_node_id: NYC, initiator_node_id: NYC, probe_rtt_ms: 70 }),
        peer({ peer_node_id: SHA, initiator_node_id: SHA, transport: "vless", listener_id: "lax-vless", probe_rtt_ms: 142 }),
        peer({ peer_node_id: TYO, initiator_node_id: TYO, probe_rtt_ms: 101 }),
      ]),
      report(SHA, [
        peer({ initiator_node_id: SHA, transport: "vless", listener_id: "lax-vless", candidate_id: "sha-to-lax", probe_rtt_ms: 139 }),
        peer({ peer_node_id: FRA, initiator_node_id: SHA, listener_id: "fra-udp", candidate_id: "sha-to-fra", state: "unreachable", reason: "authentication_failed", probe_fresh: false, probe_rtt_ms: null }),
      ]),
      report(FRA, [peer({ peer_node_id: NYC, initiator_node_id: NYC, listener_id: "fra-udp", probe_rtt_ms: 80 })]),
    ],
  }
  return { nodes, connections, runtimes }
}
