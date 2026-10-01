import assert from "node:assert/strict"
import { test } from "node:test"

import type { NodeEndpointDiscovery } from "@meshway/control-client"

import { zNodeEndpointDiscovery } from "../../../packages/control-client/src/generated/zod.gen.ts"
import { discoveryLabel, observationUsable } from "./endpoint-discovery-model.ts"

const now = Date.parse("2026-10-01T08:00:00Z")

function firstObservation(current: NodeEndpointDiscovery) {
  const socket = current.sockets[0]
  assert.ok(socket)
  const observation = socket.observations[0]
  assert.ok(observation)
  return observation
}

function report(patch: Partial<NodeEndpointDiscovery> = {}): NodeEndpointDiscovery {
  return zNodeEndpointDiscovery.parse({
    node_id: "fixture-node",
    epoch: 2,
    sequence: 1,
    revision: 7,
    digest: "a".repeat(64),
    state: "observed",
    reported_at: new Date(now).toISOString(),
    current: true,
    stale: false,
    sockets: [{
      resource_kind: "listener",
      resource_id: "fixture-udp",
      generation: 3,
      socket_id: 1,
      bound_address: "[::]:12345",
      source: "managed_udp_socket_stun",
      observations: [{
        observer: "[2001:db8::1]:3478",
        outcome: "mapped",
        mapped_endpoint: "[2001:db8::2]:12345",
        rtt_ms: 12,
        observed_at: new Date(now).toISOString(),
        expires_at: new Date(now + 30_000).toISOString(),
      }],
    }],
    ...patch,
  })
}

test("missing and null timestamp reports remain unknown, without inventing a socket generation", () => {
  assert.equal(discoveryLabel(undefined, now), "尚无报告")
  const awaiting = report({ state: "awaiting_report", reported_at: null, epoch: 0, sequence: 0, revision: null, digest: null, current: false, stale: true, sockets: [] })
  assert.equal(discoveryLabel(awaiting, now), "尚无报告")
  assert.equal(awaiting.sockets.length, 0)
})

test("the exact expiry deadline invalidates candidates even without a new server read", () => {
  const current = report()
  const observation = firstObservation(current)
  assert.equal(observationUsable(current, observation, now + 29_999), true)
  assert.equal(discoveryLabel(current, now + 29_999), "未验证候选观测")
  assert.equal(observationUsable(current, observation, now + 30_000), false)
  assert.equal(discoveryLabel(current, now + 30_000), "观测已失效")
})

test("config changes, stale flags, withdrawn generations and failures never yield a usable mapping", () => {
  for (const patch of [{ current: false }, { stale: true }, { state: "withdrawn" }, { state: "expired" }, { state: "error" }, { reported_at: null }] satisfies Partial<NodeEndpointDiscovery>[]) {
    const changed = report(patch)
    assert.equal(observationUsable(changed, firstObservation(changed), now), false)
  }
  const current = report()
  const observation = firstObservation(current)
  assert.equal(observationUsable(current, { ...observation, outcome: "timeout", mapped_endpoint: null }, now), false)
  assert.equal(observationUsable(current, { ...observation, observed_at: new Date(now + 1_000).toISOString() }, now), false)
})

test("generated schemas reject invalid timestamps, field types and unrecognized observation sources", () => {
  const current = report()
  assert.equal(zNodeEndpointDiscovery.safeParse({ ...current, reported_at: "invalid-time" }).success, false)
  assert.equal(zNodeEndpointDiscovery.safeParse({ ...current, epoch: "2" }).success, false)
  assert.equal(zNodeEndpointDiscovery.safeParse({ ...current, sockets: [{ ...current.sockets[0], source: "independent_probe" }] }).success, false)
  assert.equal(zNodeEndpointDiscovery.safeParse({ ...current, sockets: [{ ...current.sockets[0], observations: [{ ...firstObservation(current), expires_at: "invalid-time" }] }] }).success, false)
})
