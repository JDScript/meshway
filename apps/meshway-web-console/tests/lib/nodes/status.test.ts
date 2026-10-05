import assert from "node:assert/strict"
import { test } from "vitest"

import {
  lastSeen,
  mostUrgentTone,
  nodeStatus,
  statusTone,
} from "@/lib/nodes/status"

const ON = {
  enabled: true,
  controlKeyRevokedAt: null,
  dataplaneKeyRevokedAt: null,
}

test("a node's status is its report's freshness", () => {
  assert.equal(nodeStatus(ON, { freshness: "current" }), "current")
  assert.equal(nodeStatus(ON, { freshness: "expired" }), "expired")
  // A peer the inventory does not list: its report's alone.
  assert.equal(nodeStatus(null, { freshness: "superseded" }), "superseded")
})

test("turned off or with a revoked key, whatever its report says", () => {
  assert.equal(
    nodeStatus({ ...ON, enabled: false }, { freshness: "current" }),
    "disabled"
  )
  assert.equal(
    nodeStatus(
      { ...ON, dataplaneKeyRevokedAt: "2026-10-01T00:00:00Z" },
      { freshness: "current" }
    ),
    "revoked"
  )
})

test("a node the State does not name is unknown", () => {
  assert.equal(nodeStatus(ON, undefined), "unknown")
})

test("green online, orange to look at, red gone, grey off", () => {
  assert.equal(statusTone.current, "online")
  for (const status of [
    "awaiting_report",
    "superseded",
    "config_mismatch",
  ] as const)
    assert.equal(statusTone[status], "attention")
  assert.equal(statusTone.expired, "offline")
  assert.equal(statusTone.revoked, "offline")
  assert.equal(statusTone.disabled, "inactive")
  assert.equal(statusTone.unknown, "inactive")
})

test("last seen: the last report, else the last sign-in", () => {
  assert.equal(
    lastSeen({ receivedAt: "2026-10-04T10:00:00Z", lastAuthenticatedAt: null }),
    "2026-10-04T10:00:00Z"
  )
  assert.equal(
    lastSeen({ receivedAt: null, lastAuthenticatedAt: "2026-10-04T09:00:00Z" }),
    "2026-10-04T09:00:00Z"
  )
  assert.equal(lastSeen(undefined), null)
})

test("nodes shown as one take the tone most in need of a look", () => {
  assert.equal(mostUrgentTone(["current", "expired", "superseded"]), "offline")
  assert.equal(mostUrgentTone(["current", "config_mismatch"]), "attention")
  assert.equal(mostUrgentTone(["disabled", "current"]), "online")
  assert.equal(mostUrgentTone(["disabled", "unknown"]), "inactive")
  assert.equal(mostUrgentTone([]), "inactive")
})
