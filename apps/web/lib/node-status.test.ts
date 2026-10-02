import assert from "node:assert/strict"
import { test } from "node:test"

import type { NodeSummary } from "@meshway/control-client"

import { carriersByNode, needsAttention, nextPartStatus } from "./node-status.ts"

test("background read outcomes apply in read order, not arrival order", () => {
  const rendered = { at: 1_000, error: null }
  const failed = nextPartStatus(rendered, 3_000, "HTTP 502")
  assert.equal(failed.error, "HTTP 502")
  // A slower, older success arriving later does not hide the newer failure.
  assert.equal(nextPartStatus(failed, 2_000, null).error, "HTTP 502")
  // A newer success clears it; a late older failure cannot bring it back.
  const recovered = nextPartStatus(failed, 4_000, null)
  assert.equal(recovered.error, null)
  assert.equal(nextPartStatus(recovered, 3_500, "HTTP 502").error, null)
})

const summary = (patch: Partial<NodeSummary>): NodeSummary => ({
  id: "n",
  name: "n",
  revoked_at: null,
  acknowledged_revision: 3,
  configuration_revision: 3,
  agent_version: "test",
  country: null,
  last_seen: "2026-01-01T00:00:00Z",
  registered_at: "2026-01-01T00:00:00Z",
  mesh_ipv4: "100.64.0.1",
  mesh_ipv6: "fd00::1",
  ...patch,
})

test("attention: pending receipts and long silence, never revoked nodes", () => {
  const seen = Date.parse("2026-01-01T00:00:00Z")
  assert.equal(needsAttention(summary({}), seen + 60_000), false)
  assert.equal(needsAttention(summary({ acknowledged_revision: 2 }), seen), true)
  assert.equal(needsAttention(summary({ acknowledged_revision: 0 }), seen), true)
  assert.equal(needsAttention(summary({}), seen + 2 * 60 * 60 * 1000), true)
  assert.equal(needsAttention(summary({ acknowledged_revision: 0, revoked_at: "2026-01-02T00:00:00Z" }), seen), false)
})

test("carriers are counted from the saved plan per node", () => {
  const carriers = carriersByNode({
    listeners: [{ node_id: "a", transport: "vless" }, { node_id: "a", transport: "native_udp" }],
    candidates: [{ dialer_node_id: "b" }, { dialer_node_id: "b" }],
  })
  assert.deepEqual(carriers.get("a"), { listeners: ["vless", "native_udp"], outgoing: 0 })
  assert.deepEqual(carriers.get("b"), { listeners: [], outgoing: 2 })
  assert.equal(carriers.get("c"), undefined)
})
