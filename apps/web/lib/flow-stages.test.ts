import assert from "node:assert/strict"
import { test } from "node:test"

import type { PreviewFlowResponse } from "@meshway/control-client"

import { flowStages, matchedDirection } from "./flow-stages.ts"
import type { Adjacency, ChannelStatus, TopologyObservation } from "./topology-model.ts"

const NYC = "n".repeat(64)
const LAX = "l".repeat(64)
const SHA = "s".repeat(64)

function adjacency(a: string, b: string, status: ChannelStatus): Adjacency {
  const nodeIds: [string, string] = a < b ? [a, b] : [b, a]
  return { key: nodeIds.join("|"), nodeIds, channels: [], status, ambiguousProof: false }
}

const observation = (adjacencies: Adjacency[]): TopologyObservation => ({
  revision: 9,
  adjacencies,
  reports: new Map(),
  unmatchedReports: 0,
})

function routed(path: string[], ruleId: string | null = "nyc-sha"): PreviewFlowResponse {
  return {
    revision: 9,
    valid: true,
    diagnostics: [],
    authorization: { action: "allow", rule_id: "edge", matched_direction: "reverse", reason: "rule edge" },
    routing: { action: "route", path, exit_node_id: null, rule_id: ruleId, reason: "matched" },
    execution_supported: true,
    execution_reason: "eligible",
  }
}

test("each planned hop is checked against observed adjacency, never inferring a direct edge", () => {
  const stages = flowStages(
    routed([NYC, LAX, SHA]),
    observation([adjacency(NYC, LAX, "confirmed"), adjacency(SHA, LAX, "one_sided")]),
    true
  )
  const neighbors = stages.find((stage) => stage.key === "neighbors")
  assert.ok(neighbors?.key === "neighbors")
  assert.equal(neighbors.status, "observed")
  assert.deepEqual(
    neighbors.hops.map((hop) => [hop.from, hop.to, hop.adjacency]),
    [
      [NYC, LAX, "confirmed"],
      [LAX, SHA, "one_sided"],
    ]
  )

  const direct = flowStages(routed([NYC, SHA]), observation([adjacency(NYC, LAX, "confirmed")]), true)
  const hop = direct.find((stage) => stage.key === "neighbors")
  assert.ok(hop?.key === "neighbors")
  assert.equal(hop.status, "missing")
  assert.equal(hop.hops[0]?.adjacency, null, "no configured NYC–SHA channel")
})

test("without runtime reads, adjacency is unknown rather than missing", () => {
  const stages = flowStages(routed([NYC, LAX]), observation([adjacency(NYC, LAX, "confirmed")]), false)
  assert.equal(stages.find((stage) => stage.key === "neighbors")?.status, "unknown")
  assert.equal(flowStages(routed([NYC, LAX]), null, true).find((stage) => stage.key === "neighbors")?.status, "unknown")
})

test("access, route and execution come straight from the controller preview", () => {
  const rejected: PreviewFlowResponse = {
    ...routed([]),
    authorization: { action: "reject", rule_id: null, matched_direction: null, reason: "default reject" },
    routing: { action: "reject", path: [], exit_node_id: null, rule_id: null, reason: "default reject" },
    execution_supported: false,
    execution_reason: "not authorized",
  }
  const stages = flowStages(rejected, observation([]), true)
  assert.deepEqual(
    stages.map((stage) => [stage.key, stage.status]),
    [
      ["access", "rejected"],
      ["route", "rejected"],
      ["neighbors", "unknown"],
      ["execution", "ineligible"],
    ]
  )
  const [access] = flowStages(routed([NYC, LAX]), null, false)
  assert.ok(access?.key === "access")
  assert.equal(access.direction, "reverse")
  assert.equal(access.ruleId, "edge")
})

test("matched direction is checked, never cast", () => {
  assert.equal(matchedDirection("forward"), "forward")
  assert.equal(matchedDirection("reverse"), "reverse")
  assert.equal(matchedDirection("sideways"), null)
  assert.equal(matchedDirection(null), null)
})
