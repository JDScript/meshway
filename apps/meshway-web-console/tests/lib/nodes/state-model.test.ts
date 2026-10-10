import assert from "node:assert/strict"
import type { NodeState, StateView } from "@meshway/control-client"
import { test } from "vitest"

import {
  candidateExpired,
  configurationFacts,
  configurationSync,
  desiredAgainst,
  listenerRuntime,
  nodeLinks,
  nodePaths,
  otherSockets,
  reportOf,
  stateOf,
  unconfiguredListeners,
} from "@/lib/nodes/state-model"

const A = "11111111-1111-4111-8111-111111111111"
const B = "22222222-2222-4222-8222-222222222222"
const C = "33333333-3333-4333-8333-333333333333"
const INSTANCE = "44444444-4444-4444-8444-444444444444"
const SERIAL = 1
const DIGEST = "a".repeat(64)
const T0 = "2026-10-03T10:00:00Z"
const T1 = "2026-10-03T10:05:00Z"

const ref = (revision: number) => ({
  revision,
  serial: SERIAL,
  digest: DIGEST,
})

const REPORT: NodeState = {
  capabilities: {},
  configuration: { saved: ref(4), applied: ref(3) },
  links: {},
  listeners: {
    udp: { phase: "listening", socketIds: ["s1", "gone"] },
  },
  paths: {},
  probes: [],
  problems: [],
  selection: {},
  sockets: {
    s1: {
      boundAddress: "0.0.0.0:8421",
      endpoints: [],
      generation: 1,
      network: "udp",
      resource: { kind: "listener", key: "udp" },
    },
    d1: {
      boundAddress: "0.0.0.0:50000",
      endpoints: [],
      generation: 2,
      network: "udp",
      resource: { kind: "dial", key: "native_udp" },
    },
  },
  system: { agentVersion: "0.1.0", arch: "x86_64", hostname: "a", os: "linux" },
}

const link = (a: string, b: string, linkId: string) => ({
  a,
  b,
  linkId,
  evidence: "matched" as const,
  initiatorNodeId: a,
  phase: "authenticated" as const,
  reports: [],
  transport: "native_udp",
})

const STATE: StateView = {
  asOf: T1,
  configurationRevision: 4,
  stateEpoch: INSTANCE,
  sequence: 9,
  networkId: INSTANCE,
  nodes: {
    [A]: {
      hostname: "a",
      derived: { ipInfo: {} },
      freshness: "current",
      receivedAt: T1,
      observedAt: T0,
      report: { view: "full", report: REPORT },
    },
    [B]: {
      hostname: "b",
      derived: { ipInfo: {} },
      freshness: "awaiting_report",
    },
  },
  links: [link(B, A, "l2"), link(A, C, "l1"), link(B, C, "l3")],
  paths: [
    {
      pathId: "p1",
      reporter: { nodeId: A, instanceId: INSTANCE, reportSequence: 1 },
      state: {
        evidence: "sent",
        expiresAt: T1,
        nodes: [A, B],
        observedAt: T0,
        netmap: ref(3),
        reporterRole: "origin",
      },
    },
  ],
}

test("a node's report, its view and time", () => {
  assert.equal(reportOf(stateOf(STATE, A)).view, "full")
  assert.equal(reportOf(stateOf(STATE, B)).view, "none")
  assert.equal(stateOf(STATE, C), undefined)
})

test("configuration facts against the server's netmap for the node", () => {
  assert.deepEqual(configurationFacts(REPORT.configuration, ref(4)), {
    saved: ref(4),
    applied: ref(3),
    progress: null,
    current: false,
  })
  assert.equal(configurationFacts({}, undefined).current, null)
  assert.equal(
    configurationFacts({ applied: ref(3) }, null).current,
    null,
    "the server holding none says nothing either way"
  )
})

test("where a node's configuration stands, as one fact", () => {
  const progress = (
    revision: number,
    phase: "applying" | "failed" | "applied"
  ) => ({
    target: ref(revision),
    phase,
    observedAt: T1,
  })
  assert.deepEqual(configurationSync(null, ref(4)), { kind: "unknown" })
  assert.deepEqual(configurationSync({}, ref(4)), { kind: "none" })
  assert.deepEqual(configurationSync({ applied: ref(4) }, ref(4)), {
    kind: "current",
    applied: ref(4),
  })
  assert.deepEqual(configurationSync({ applied: ref(3) }, ref(4)), {
    kind: "behind",
    applied: ref(3),
    assigned: ref(4),
  })
  // Unread, or none held: only what the node reports.
  for (const assigned of [undefined, null])
    assert.deepEqual(configurationSync({ applied: ref(3) }, assigned), {
      kind: "applied",
      applied: ref(3),
    })
  // A netmap on its way, or refused, says more than the one applied.
  assert.deepEqual(
    configurationSync(
      { applied: ref(3), progress: progress(4, "applying") },
      ref(4)
    ),
    { kind: "applying", target: ref(4) }
  )
  assert.deepEqual(
    configurationSync(
      { applied: ref(3), progress: progress(4, "failed") },
      ref(4)
    ),
    { kind: "failed", target: ref(4) }
  )
  assert.deepEqual(
    configurationSync(
      { applied: ref(4), progress: progress(4, "applied") },
      ref(4)
    ),
    { kind: "current", applied: ref(4) }
  )
})

test("up to date is the node's netmap, not the network's revision", () => {
  // A later revision that leaves this node's netmap as it was (a comment,
  // another node) does not make it behind: the server still holds r3 #1.
  assert.deepEqual(configurationSync({ applied: ref(3) }, ref(3)), {
    kind: "current",
    applied: ref(3),
  })
  // Same revision, new content (a key rotated, a peer's capability changed):
  // the serial moved, so the node is behind.
  const next = { ...ref(3), serial: SERIAL + 1, digest: "b".repeat(64) }
  assert.deepEqual(configurationSync({ applied: ref(3) }, next), {
    kind: "behind",
    applied: ref(3),
    assigned: next,
  })
})

test("listener runtime names its sockets; others are dial sockets", () => {
  const runtime = listenerRuntime(REPORT, "udp")
  assert.deepEqual(
    runtime?.sockets.map((entry) => [entry.id, entry.socket !== null]),
    [
      ["s1", true],
      ["gone", false],
    ]
  )
  assert.equal(listenerRuntime(REPORT, "tcp"), null)
  assert.deepEqual(
    otherSockets(REPORT).map((entry) => entry.id),
    ["d1"]
  )
  assert.deepEqual(unconfiguredListeners(REPORT, ["vless"]), ["udp"])
})

test("a link says how the node acquired it, by its own report", () => {
  const a = STATE.nodes[A]
  assert.ok(a)
  const reported: StateView = {
    ...STATE,
    nodes: {
      ...STATE.nodes,
      [A]: {
        ...a,
        report: {
          view: "full",
          report: {
            ...REPORT,
            links: {
              [B]: [
                {
                  linkId: "l2",
                  phase: "authenticated",
                  transport: "native_udp",
                  socketId: "s1",
                  initiatorNodeId: A,
                  businessDependencies: 0,
                  observationIds: [],
                  observedAt: T0,
                  expiresAt: T1,
                  acquisition: { type: "localEndpoint" },
                },
              ],
            },
          },
        },
      },
    },
  }
  assert.deepEqual(
    nodeLinks(reported, A).map((entry) => [entry.linkId, entry.acquisition]),
    [
      ["l2", { type: "localEndpoint" }],
      // Its report does not name this one.
      ["l1", null],
    ]
  )
  // Another node's view of the same link is its own report's, unseen here.
  assert.equal(nodeLinks(reported, B)[0]?.acquisition, null)
})

test("links and paths through a node", () => {
  assert.deepEqual(
    nodeLinks(STATE, A).map((entry) => [entry.peerId, entry.linkId]),
    [
      [B, "l2"],
      [C, "l1"],
    ]
  )
  assert.equal(nodePaths(STATE, B).length, 1)
  assert.equal(nodePaths(STATE, C).length, 0)
  assert.deepEqual(nodeLinks(undefined, A), [])
})

test("an endpoint past its expiry is expired", () => {
  const candidate = {
    address: "203.0.113.1:8421",
    observedAt: T0,
    expiresAt: T1,
    origin: { source: "stun" as const, observer: "198.51.100.1:3478" },
  }
  assert.equal(candidateExpired(candidate, Date.parse(T0)), false)
  assert.equal(candidateExpired(candidate, Date.parse(T1)), true)
})

test("the desired target against what the node reports", () => {
  const target = ref(4)
  const other = { ...ref(4), digest: "b".repeat(64) }
  assert.equal(desiredAgainst(null, { applied: target }), "none")
  assert.equal(desiredAgainst(target, null), "unreported")
  assert.equal(
    desiredAgainst(target, { saved: target, applied: target }),
    "applied"
  )
  assert.equal(
    desiredAgainst(target, { saved: target, applied: ref(3) }),
    "saved"
  )
  // The same revision in another slice is another target.
  assert.equal(
    desiredAgainst(target, { saved: other, applied: other }),
    "elsewhere"
  )
  assert.equal(desiredAgainst(target, {}), "elsewhere")
  assert.equal(
    desiredAgainst(target, {
      applied: { ...target, serial: SERIAL + 1 },
    }),
    "elsewhere"
  )
})
