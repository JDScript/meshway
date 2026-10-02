import assert from "node:assert/strict"
import { test } from "node:test"

import type { NodeEndpointDiscovery } from "@meshway/control-client"

import {
  discoveryStatus,
  observationState,
  socketsByResource,
} from "./discovery-model.ts"

const NOW = Date.parse("2026-10-02T10:00:00Z")
const entry = (
  patch: Partial<NodeEndpointDiscovery> = {}
): NodeEndpointDiscovery => ({
  current: true,
  digest: "d".repeat(64),
  epoch: 1,
  node_id: "a".repeat(64),
  reported_at: new Date(NOW - 2000).toISOString(),
  revision: 3,
  sequence: 4,
  stale: false,
  state: "observed",
  sockets: [
    {
      bound_address: "0.0.0.0:41000",
      generation: 1,
      observations: [],
      resource_id: "a-udp",
      resource_kind: "listener",
      socket_id: 0,
      source: "managed_udp_socket_stun",
    },
    {
      bound_address: "0.0.0.0:53122",
      generation: 1,
      observations: [],
      resource_id: "a-to-b",
      resource_kind: "outgoing_candidate",
      socket_id: 1,
      source: "managed_udp_socket_stun",
    },
  ],
  ...patch,
})

test("a report that does not stand is not shown as observed", () => {
  assert.equal(discoveryStatus(undefined), "unreported")
  assert.equal(discoveryStatus(entry()), "observed")
  assert.equal(discoveryStatus(entry({ stale: true })), "stale")
  assert.equal(discoveryStatus(entry({ current: false })), "outdated")
  assert.equal(discoveryStatus(entry({ state: "disabled" })), "disabled")
  assert.equal(
    discoveryStatus(entry({ state: "withdrawn", stale: true })),
    "withdrawn"
  )
})

test("a mapping expires at its own time and with its report", () => {
  const observation = {
    expires_at: new Date(NOW + 5000).toISOString(),
    mapped_endpoint: "203.0.113.7:41000",
    observed_at: new Date(NOW - 1000).toISOString(),
    observer: "198.51.100.1:3478",
    outcome: "mapped" as const,
    rtt_ms: 12,
  }
  assert.equal(observationState(observation, true, NOW), "mapped")
  assert.equal(observationState(observation, true, NOW + 5000), "expired")
  assert.equal(observationState(observation, false, NOW), "expired")
  assert.equal(
    observationState(
      { ...observation, outcome: "timeout", mapped_endpoint: null },
      true,
      NOW
    ),
    "failed"
  )
})

test("dial sockets stay with their candidate, never as listeners", () => {
  const groups = socketsByResource(entry())
  assert.deepEqual([...groups.listeners.keys()], ["a-udp"])
  assert.deepEqual([...groups.dialers.keys()], ["a-to-b"])
})
