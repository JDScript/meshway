import assert from "node:assert/strict"
import test from "node:test"

import { topologyFlowHref, topologyInitialFlow } from "./flow-navigation.ts"

test("flow handoff preserves encoded endpoints and ignores external hints for a node target", () => {
  const href = topologyFlowHref({ sourceNodeId: "source / one", targetNodeId: "target&two", domain: "ignored.example" })
  const url = new URL(href, "http://localhost")
  assert.equal(url.hash, "#flow-check")
  const restored = topologyInitialFlow(Object.fromEntries(url.searchParams))
  assert.equal(restored.sourceNodeId, "source / one")
  assert.equal(restored.targetNodeId, "target&two")
  assert.equal(restored.domain, "")
})

test("external target context and legacy node links survive handoff without choosing another identity", () => {
  const url = new URL(topologyFlowHref({ sourceNodeId: "missing-node", domain: "www.example.com", ip: "203.0.113.1", country: "JP" }), "http://localhost")
  assert.deepEqual(topologyInitialFlow(Object.fromEntries(url.searchParams)), {
    sourceNodeId: "missing-node", targetNodeId: "", domain: "www.example.com", ip: "203.0.113.1", country: "JP",
  })
  assert.equal(topologyInitialFlow({ node: "old-link" }).sourceNodeId, "old-link")
  assert.equal(topologyInitialFlow({ node: "old-link", src: "explicit-source" }).sourceNodeId, "explicit-source")
  assert.equal(topologyInitialFlow({ src: ["ambiguous", "source"] }).sourceNodeId, "")
})
