import assert from "node:assert/strict"
import { test } from "node:test"

import {
  deploymentOf,
  nodePhase,
  publicationSettled,
  type NodeReport,
} from "./progress.ts"

const report = (over: Partial<NodeReport> = {}): NodeReport => ({
  saved: false,
  applied: false,
  failed: false,
  freshness: "current",
  ...over,
})

test("a node's phase is the furthest its receipts reached, a failure first", () => {
  assert.equal(nodePhase(report()), "unreported")
  assert.equal(nodePhase(report({ saved: true })), "saved")
  assert.equal(nodePhase(report({ saved: true, applied: true })), "applied")
  assert.equal(nodePhase(report({ saved: true, failed: true })), "failed")
})

test("a publication is settled when withdrawn or when no node is still on its way", () => {
  assert.equal(publicationSettled({ withdrawnAt: null, nodes: {} }), true)
  assert.equal(
    publicationSettled({
      withdrawnAt: null,
      nodes: { a: report({ applied: true }), b: report({ saved: true }) },
    }),
    false
  )
  assert.equal(
    publicationSettled({
      withdrawnAt: null,
      nodes: {
        a: report({ applied: true }),
        b: report({ failed: true }),
        c: report({ freshness: "superseded" }),
      },
    }),
    true
  )
  assert.equal(
    publicationSettled({
      withdrawnAt: "2026-10-03T12:00:00Z",
      nodes: { a: report() },
    }),
    true
  )
})

test("an expired or waiting node is not settled", () => {
  for (const freshness of ["expired", "awaitingInstance"] as const)
    assert.equal(
      publicationSettled({
        withdrawnAt: null,
        nodes: { a: report({ saved: true, freshness }) },
      }),
      false
    )
})

test("no node deployment only when there are no targets and no publications", () => {
  assert.equal(deploymentOf({ targetCount: 0, publicationIds: [] }), "none")
  assert.equal(
    deploymentOf({ targetCount: 2, publicationIds: [] }),
    "unpublished"
  )
  assert.equal(
    deploymentOf({ targetCount: 0, publicationIds: ["p"] }),
    "published"
  )
  assert.equal(
    deploymentOf({ targetCount: 3, publicationIds: ["p", "q"] }),
    "published"
  )
})
