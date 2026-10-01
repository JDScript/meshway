import assert from "node:assert/strict"
import { test } from "node:test"

import type { ConnectionNode, ConnectionPlan } from "@meshway/control-client"

// Parse with the generated runtime schema, exactly as the SDK does, so the
// test sees the SDK's real object key order rather than a hand-written one.
import {
  zConnectionNode,
  zConnectionPlan,
} from "../../../packages/control-client/src/generated/zod.gen.ts"
import {
  bindProblem,
  draftKey,
  fromPlan,
  hostProblem,
  planKey,
  toPlan,
  validateConnections,
} from "./connections-model.ts"

const plan: ConnectionPlan = {
  listeners: [
    {
      id: "lax-in",
      node_id: "lax",
      transport: "native_udp",
      bind: "0.0.0.0:51820",
      advertised_endpoints: [
        { host: "lax.example.net", port: 51820 },
        { host: "203.0.113.7", port: 51820 },
      ],
      credential_ref: null,
    },
  ],
  candidates: [
    {
      id: "sha-to-lax",
      dialer_node_id: "sha",
      listener_id: "lax-in",
      endpoint: { host: "lax.example.net", port: 51820 },
      priority: 10,
      credential_ref: null,
    },
  ],
}

function node(id: string, publicKey: string | null, revoked = false): ConnectionNode {
  return zConnectionNode.parse({
    node: {
      id,
      name: id.toUpperCase(),
      mesh_ipv4: "100.64.0.1",
      mesh_ipv6: "fd00::1",
      country: null,
      agent_version: "0.0.1",
      registered_at: "2026-09-30T00:00:00Z",
      last_seen: "2026-09-30T00:00:00Z",
      revoked_at: revoked ? "2026-09-30T00:00:00Z" : null,
      acknowledged_revision: 0,
      configuration_revision: 1,
    },
    public_key: publicKey,
    capabilities:
      publicKey === null
        ? []
        : [{ transport: "native_udp", can_dial: true, can_accept: true, semantics: "datagram" }],
    acknowledged_revision: 0,
    acknowledged_digest: null,
    acknowledged_at: null,
  })
}

const key = "a".repeat(64)

test("plan identity ignores DTO key order and stored sort order", () => {
  const parsed = zConnectionPlan.parse(plan)
  const reversed: ConnectionPlan = {
    ...plan,
    listeners: plan.listeners.map((listener) => ({
      ...listener,
      advertised_endpoints: [...listener.advertised_endpoints].reverse(),
    })),
  }
  assert.equal(planKey(parsed), planKey(plan))
  assert.equal(planKey(reversed), planKey(plan))
  assert.equal(draftKey(fromPlan(parsed)), planKey(plan))
  assert.deepEqual(toPlan(fromPlan(plan)), plan)
})

test("a valid plan has no local issues", () => {
  const nodes = [node("lax", key), node("sha", key.replace("a", "b"))]
  assert.deepEqual(validateConnections(fromPlan(plan), nodes), [])
})

test("missing keys, revoked dialers and self-dial are flagged, never dropped", () => {
  const draft = fromPlan(plan)
  const fields = (nodes: ConnectionNode[]) =>
    validateConnections(draft, nodes).map((issue) => issue.field)
  assert.ok(fields([node("lax", null), node("sha", key)]).includes("listeners.node_id"))
  assert.ok(fields([node("lax", key), node("sha", key, true)]).includes("candidates.dialer_node_id"))
  const selfDial = fromPlan({
    ...plan,
    candidates: plan.candidates.map((candidate) => ({ ...candidate, dialer_node_id: "lax" })),
  })
  assert.ok(
    validateConnections(selfDial, [node("lax", key)]).some(
      (issue) => issue.field === "candidates.dialer_node_id"
    )
  )
  assert.equal(draft.candidates.length, 1)
})

test("advertised hosts reject wildcards; bind accepts them", () => {
  assert.notEqual(hostProblem("0.0.0.0"), null)
  assert.notEqual(hostProblem("::"), null)
  assert.notEqual(hostProblem("224.0.0.1"), null)
  assert.equal(hostProblem("2001:db8::1"), null)
  assert.equal(hostProblem("LAX.example.net."), null)
  assert.equal(bindProblem("0.0.0.0:51820"), null)
  assert.equal(bindProblem("[::]:443"), null)
  assert.notEqual(bindProblem("0.0.0.0:0"), null)
  assert.notEqual(bindProblem("lax.example.net:443"), null)
})

test("VLESS needs profile refs on both sides; native carriers must not have one", () => {
  const vless = fromPlan({
    listeners: plan.listeners.map((listener) => ({ ...listener, transport: "vless" })),
    candidates: plan.candidates,
  })
  const vlessNode = (id: string) => ({
    ...node(id, key),
    capabilities: [{ transport: "vless" as const, can_dial: true, can_accept: true, semantics: "stream" as const }],
  })
  const fields = validateConnections(vless, [vlessNode("lax"), vlessNode("sha")]).map((i) => i.field)
  assert.ok(fields.includes("listeners.credential_ref"))
  assert.ok(fields.includes("candidates.credential_ref"))
  const native = fromPlan({
    ...plan,
    listeners: plan.listeners.map((listener) => ({ ...listener, credential_ref: "profile" })),
  })
  assert.ok(
    validateConnections(native, [node("lax", key), node("sha", key)]).some(
      (issue) => issue.field === "listeners.credential_ref"
    )
  )
})
