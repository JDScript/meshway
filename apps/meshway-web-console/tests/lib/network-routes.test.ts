import assert from "node:assert/strict"
import type {
  AggregatedPath,
  EvaluationResponse,
} from "@meshway/control-client"
import { test } from "vitest"

import {
  hopCount,
  plannedRoute,
  reportedFlows,
  routeEdges,
  routeTree,
  viaOf,
} from "@/lib/network-routes"

const A = "11111111-1111-4111-8111-111111111111"
const R = "22222222-2222-4222-8222-222222222222"
const B = "33333333-3333-4333-8333-333333333333"
const REQUEST = "44444444-4444-4444-8444-444444444444"

const plan = (
  target: string,
  nodes: string[],
  allowed = true
): EvaluationResponse => ({
  requestId: REQUEST,
  revision: 3,
  draft: false,
  evaluation: {
    source: A,
    destination: { type: "node", nodeId: target },
    access: { allowed, grants: [] },
    profile: {
      name: "mesh",
      assignment: null,
      allowed: [],
      locked: false,
      requested: false,
      capture: {
        managedIpv4: 0,
        managedIpv6: 0,
        sinkIpv6: 0,
        routes: 0,
        unavailable: null,
      },
    },
    route: null,
    outcome: allowed
      ? { type: "permitted" }
      : { type: "unavailable", reason: "unauthorized" },
    targets: [{ nodeId: target, paths: nodes.length ? 1 : 0, problem: null }],
    paths: nodes.length
      ? [
          {
            nodes,
            // The controller keys edges without direction.
            edges: nodes.slice(1).map((to, index) => ({
              a: to,
              b: nodes[index] ?? "",
              topology: index === 0 ? null : "/topology/1",
            })),
          },
        ]
      : [],
  },
})

test("a plan becomes hops in the path's order, with each edge's rule", () => {
  const route = plannedRoute(B, plan(B, [A, R, B]))
  assert.equal(route.status, "planned")
  assert.deepEqual(route.status === "planned" ? route.hops : null, [
    { from: A, to: R, topology: null },
    { from: R, to: B, topology: "/topology/1" },
  ])
  assert.deepEqual(viaOf(route), [R])
  assert.equal(hopCount(route), 2)
  assert.equal(hopCount(plannedRoute(B, plan(B, []))), null)
})

test("the tree places each target by hops, under the node it is reached through", () => {
  const routes = [
    plannedRoute(R, plan(R, [A, R])),
    plannedRoute(B, plan(B, [A, R, B], false)),
  ]
  const tree = routeTree(routes)
  assert.equal(tree.depth.get(R), 1)
  assert.equal(tree.depth.get(B), 2)
  assert.equal(tree.parent.get(B), R)
  // A shared hop is one edge, used by both destinations; access is per destination.
  const edges = routeEdges(routes)
  assert.deepEqual(
    edges.map((edge) => [edge.key, edge.targets, edge.authorized]),
    [
      [`${A}>${R}`, [R, B], true],
      [`${R}>${B}`, [B], false],
    ]
  )
})

test("reported paths give each pair the directions traffic takes; expired reports none", () => {
  const path = (nodes: string[], expiresAt: string): AggregatedPath => ({
    pathId: nodes.join(),
    reporter: { nodeId: nodes[0] ?? A, instanceId: REQUEST, reportSequence: 1 },
    state: {
      nodes,
      evidence: "sent",
      reporterRole: "origin",
      netmap: { revision: 1, serial: 1, digest: "d".repeat(64) },
      observedAt: "2026-10-08T00:00:00Z",
      expiresAt,
    },
  })
  const flows = reportedFlows(
    [
      path([A, R, B], "2026-10-08T00:02:00Z"),
      path([B, R], "2026-10-08T00:00:30Z"),
    ],
    "2026-10-08T00:01:00Z"
  )
  const pair = (x: string, y: string) => (x < y ? `${x}|${y}` : `${y}|${x}`)
  assert.deepEqual([...(flows.get(pair(A, R)) ?? [])], [`${A}>${R}`])
  assert.deepEqual([...(flows.get(pair(R, B)) ?? [])], [`${R}>${B}`])
})
