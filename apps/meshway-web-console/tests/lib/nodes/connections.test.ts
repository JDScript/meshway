import assert from "node:assert/strict"
import type { AggregatedLink } from "@meshway/control-client"
import { test } from "vitest"

import { neighborConnections, nodeConnections } from "@/lib/nodes/connections"

const A = "11111111-1111-4111-8111-111111111111"
const R = "22222222-2222-4222-8222-222222222222"
const B = "33333333-3333-4333-8333-333333333333"
const link = (
  linkId: string,
  fields: Partial<AggregatedLink> = {}
): AggregatedLink => ({
  a: A,
  b: R,
  linkId,
  phase: "authenticated",
  evidence: "matched",
  transport: "native_udp",
  initiatorNodeId: A,
  reports: [],
  ...fields,
})

test("a current authenticated report counts even when only one side is visible", () => {
  assert.equal(
    neighborConnections([link("visible", { evidence: "oneSided" })])[0]?.status,
    "connected"
  )
})

test("parallel channels and opposite dial directions remain one adjacent peer", () => {
  const connections = neighborConnections([
    link("udp"),
    link("tcp", { a: R, b: A, initiatorNodeId: R, transport: "native_tcp" }),
  ])
  assert.equal(connections.length, 1)
  assert.equal(connections[0]?.channels.length, 2)
  assert.deepEqual(connections[0]?.transports, ["native_tcp", "native_udp"])
})

test("a stale previous channel does not hide a current replacement or choose its transport", () => {
  const connection = neighborConnections([
    link("old", { evidence: "expired", transport: "native_tcp" }),
    link("new", { evidence: "oneSided" }),
  ])[0]
  assert.equal(connection?.status, "connected")
  assert.deepEqual(connection?.transports, ["native_udp"])
})

test("stale or conflicting evidence is unknown, not a disconnected node or a confirmed channel", () => {
  for (const evidence of ["expired", "conflicting"] as const) {
    const connection = neighborConnections([link("old", { evidence })])[0]
    assert.equal(connection?.status, "unknown")
    assert.deepEqual(connection?.transports, [])
  }
  assert.equal(
    neighborConnections([
      link("old-failure", { phase: "failed", evidence: "expired" }),
    ])[0]?.status,
    "unknown"
  )
  assert.equal(
    neighborConnections([
      link("disputed-close", { phase: "closed", evidence: "conflicting" }),
    ])[0]?.status,
    "unknown"
  )
})

test("current connection attempts are separate from established channels; closed channels leave the adjacency", () => {
  assert.equal(
    neighborConnections([link("pending", { phase: "preparing" })])[0]?.status,
    "connecting"
  )
  assert.equal(
    neighborConnections([link("failed", { phase: "failed" })])[0]?.status,
    "failed"
  )
  assert.deepEqual(
    neighborConnections([link("closed", { phase: "closed" })]),
    []
  )
})

test("A through R to B shows only R as A's adjacent peer", () => {
  const connections = nodeConnections(
    [link("ar"), link("rb", { a: R, b: B })],
    A
  )
  assert.deepEqual(
    connections.map((connection) => connection.peerId),
    [R]
  )
})
