import assert from "node:assert/strict"
import { test } from "node:test"

import type { ConnectionNode, ConnectionPlan, ConnectionsSnapshot } from "@meshway/control-client"

// Parse with the generated runtime schema, exactly as the SDK does, so the
// test sees the SDK's real object key order rather than a hand-written one.
import {
  zConnectionNode,
  zConnectionPlan,
  zConnectionsSnapshot,
} from "../../../packages/control-client/src/generated/zod.gen.ts"
import {
  autofillCandidate,
  autofillListener,
  latestSnapshot,
  bindProblem,
  composeBind,
  dialRole,
  dialRoleLabel,
  endpointHint,
  fromCandidateForm,
  newCandidateForm,
  newListenerForm,
  nodeScope,
  parseBind,
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

test("bind parts round-trip exactly, including wildcards and IPv6 brackets", () => {
  for (const bind of ["0.0.0.0:51820", "[::]:443", "192.168.1.2:9", "[fd00::2]:51820", "0.0.0.0:", "bad"]) {
    assert.equal(composeBind(parseBind(bind)), bind === "bad" ? "bad:" : bind)
  }
  assert.deepEqual(parseBind("0.0.0.0:41000"), { mode: "ipv4_any", address: "", port: "41000" })
  assert.equal(parseBind("[::]:1").mode, "ipv6_any")
})

test("advertised-host hints flag mesh addresses and bare node names, never block", () => {
  const nodes = [node("lax", key)]
  assert.match(endpointHint("100.64.0.1", nodes) ?? "", /Mesh/)
  assert.match(endpointHint("LAX", nodes) ?? "", /节点名称/)
  assert.equal(endpointHint("lax.example.net", nodes), null)
  assert.equal(endpointHint("203.0.113.7", nodes), null)
})

test("a new listener never prefills a mesh or advertised address and gets a readable ID", () => {
  const nodes = [node("lax", key)]
  const form = newListenerForm("new-0", fromPlan(plan), nodes, "lax")
  assert.equal(form.bind, "0.0.0.0:")
  assert.equal(form.endpoints[0]?.host, "")
  assert.equal(form.id, "lax-udp")
})

test("choosing a target with exactly one usable entry picks it; primary / backup follow priority", () => {
  const nodes = [node("lax", key), node("sha", key), node("nyc", key)]
  const draft = fromPlan(plan)
  const form = newCandidateForm("new-0", draft, nodes, { dialerNodeId: "nyc" })
  const picked = autofillCandidate({ ...form, targetNodeId: "lax" }, form, "new", draft, nodes)
  // lax-in advertises two endpoints: nothing is chosen silently.
  assert.equal(picked.host, "")
  assert.equal(picked.id, "nyc-to-lax-udp")
  assert.equal(picked.priority, "0")

  const single = { ...draft, listeners: draft.listeners.map((l) => ({ ...l, endpoints: l.endpoints.slice(0, 1) })) }
  const chosen = autofillCandidate({ ...form, targetNodeId: "lax" }, form, "new", single, nodes)
  assert.equal(chosen.listenerId, "lax-in")
  assert.equal(chosen.host, "lax.example.net")

  // SHA already dials LAX at priority 10: a second entry goes behind it.
  const backup = newCandidateForm("new-1", draft, nodes, { dialerNodeId: "sha", targetNodeId: "lax" })
  assert.equal(backup.priority, "20")
  const candidate = { ...fromCandidateForm(backup), host: "203.0.113.7", port: "51820", listenerId: "lax-in" }
  const withBackup = { ...draft, candidates: [...draft.candidates, candidate] }
  assert.equal(dialRoleLabel(dialRole(candidate, withBackup)), "备用 1")
  const [primary] = withBackup.candidates
  assert.ok(primary)
  assert.equal(dialRoleLabel(dialRole(primary, withBackup)), "主用")
})

test("a user-edited ID or priority is not overwritten by later choices", () => {
  const nodes = [node("lax", key), node("sha", key), node("nyc", key)]
  const draft = fromPlan(plan)
  const form = { ...newCandidateForm("new-0", draft, nodes, {}), id: "my-id", priority: "5" }
  const next = autofillCandidate({ ...form, dialerNodeId: "nyc" }, form, "new", draft, nodes)
  assert.equal(next.id, "my-id")
  assert.equal(next.priority, "5")
})

test("the per-node view splits owned listeners, outgoing and incoming dials", () => {
  const draft = fromPlan(plan)
  const lax = nodeScope(draft, "lax")
  assert.deepEqual([lax.listeners.length, lax.outgoing.length, lax.incoming.length], [1, 0, 1])
  const sha = nodeScope(draft, "sha")
  assert.deepEqual([sha.listeners.length, sha.outgoing.length, sha.incoming.length], [0, 1, 0])
})

test("a typed listener ID survives later node / transport choices; an automatic one follows them", () => {
  const nodes = [node("lax", key), node("sha", key)]
  const draft = fromPlan({ listeners: [], candidates: [] })
  const initial = newListenerForm("new-0", draft, nodes, "lax")
  assert.equal(initial.id, "lax-udp")

  const auto = autofillListener({ ...initial, nodeId: "sha" }, initial, "new", [], nodes)
  assert.equal(auto.id, "sha-udp", "still automatic: follows the node")

  const typed = autofillListener({ ...auto, id: "edge-in" }, auto, "new", [], nodes)
  assert.equal(typed.id, "edge-in", "the keystroke itself is kept")
  const later = autofillListener({ ...typed, nodeId: "lax" }, typed, "new", [], nodes)
  assert.equal(later.id, "edge-in", "and survives later choices")
  const bind = autofillListener({ ...later, bind: "0.0.0.0:443" }, later, "new", [], nodes)
  assert.equal(bind.id, "edge-in")
})

test("snapshot sources: higher revision wins, equal revision goes to the later read, nothing rolls back", () => {
  const at = (revision: number, configuration: ConnectionPlan, acknowledged = 0): ConnectionsSnapshot =>
    zConnectionsSnapshot.parse({
      revision,
      configuration,
      valid: true,
      diagnostics: [],
      nodes: [{ ...node("lax", key), acknowledged_revision: acknowledged }],
    })
  const page = { snapshot: at(4, { listeners: [], candidates: [] }), readAt: 1_000 }

  // A save adopts R+1 over the older page render; overview counts follow it.
  const saved = { snapshot: at(5, plan), readAt: 2_000 }
  const afterSave = latestSnapshot(page, saved)
  assert.equal(afterSave?.snapshot.revision, 5)
  assert.deepEqual(
    (({ listeners, outgoing, incoming }) => [listeners.length, outgoing.length, incoming.length])(
      nodeScope(fromPlan(afterSave?.snapshot.configuration ?? { listeners: [], candidates: [] }), "lax")
    ),
    [1, 0, 1]
  )

  // An explicit reload at the same revision beats the older page render.
  const reloaded = { snapshot: at(4, { listeners: [], candidates: [] }, 4), readAt: 1_500 }
  assert.equal(latestSnapshot(page, reloaded)?.snapshot.nodes[0]?.acknowledged_revision, 4)

  // A later server render or poll at the same revision (new ACK) beats the adopted save.
  const acked = { snapshot: at(5, plan, 5), readAt: 3_000 }
  assert.equal(latestSnapshot(acked, saved)?.snapshot.nodes[0]?.acknowledged_revision, 5)
  assert.equal(latestSnapshot(saved, acked)?.snapshot.nodes[0]?.acknowledged_revision, 5)

  // A lower revision never rolls back, even if read later.
  const stale = { snapshot: at(4, { listeners: [], candidates: [] }), readAt: 9_000 }
  assert.equal(latestSnapshot(stale, saved)?.snapshot.revision, 5)
  assert.equal(latestSnapshot(null, null), null)
})
