import assert from "node:assert/strict"
import { test } from "node:test"

import type { ConnectionNode, MeshAccessPolicy, MeshAccessSnapshot } from "@meshway/control-client"

import {
  ACCESS_LIMITS,
  accessKey,
  accessSaveFailure,
  formPath,
  GENERATED_GRANT_ID,
  grantChange,
  grantsFromForm,
  hasReverse,
  newGrantForm,
  newGrantId,
  readyNodes,
  removedGrantCount,
  resolveAccessNode,
  sameAccess,
  settleSave,
  toGrantForm,
  validateAccess,
  validateGrantForm,
  validatePath,
} from "./access-model.ts"

const hex = (seed: string) => seed.repeat(64).slice(0, 64)
const A = hex("a")
const B = hex("b")
const C = hex("c")
const D = hex("d")
const REVOKED = hex("e")
const KEYLESS = hex("f")
const G1 = hex("1")
const G2 = hex("2")
const G3 = hex("3")

function entry(id: string, name: string, options: { revoked?: boolean; key?: boolean } = {}): ConnectionNode {
  return {
    acknowledged_at: null,
    acknowledged_digest: null,
    acknowledged_revision: 0,
    capabilities: [],
    public_key: options.key === false ? null : `key-${name}`,
    node: {
      acknowledged_revision: 0,
      agent_version: "0.0.1",
      configuration_revision: 1,
      country: null,
      id,
      last_seen: "2026-09-30T00:00:00Z",
      mesh_ipv4: "100.96.0.1",
      mesh_ipv6: "fd00::1",
      name,
      registered_at: "2026-09-30T00:00:00Z",
      revoked_at: options.revoked ? "2026-09-30T01:00:00Z" : null,
    },
  }
}

const inventory = [
  entry(A, "nyc"),
  entry(B, "lax"),
  entry(C, "tyo"),
  entry(D, "sha"),
  entry(REVOKED, "old", { revoked: true }),
  entry(KEYLESS, "fresh", { key: false }),
]

function snapshot(revision: number, policy: MeshAccessPolicy, valid = true): MeshAccessSnapshot {
  return { revision, policy, valid, diagnostics: [] }
}

function at<T>(items: readonly T[], index: number): T {
  const item = items[index]
  assert.ok(item !== undefined)
  return item
}

const fields = (issues: { field: string }[]) => issues.map((issue) => issue.field)

test("grant IDs are generated 64-character lowercase hex", () => {
  const id = newGrantId()
  assert.match(id, GENERATED_GRANT_ID)
  assert.notEqual(newGrantId(), id)
  assert.equal(newGrantId((bytes) => bytes.fill(0xab)), "ab".repeat(32))
})

test("a path needs 2..8 chosen, unique, usable nodes", () => {
  assert.deepEqual(validatePath([A, B], inventory), [])
  assert.deepEqual(validatePath([A, B, C, D], inventory), [])
  assert.deepEqual(fields(validatePath([A], inventory)), ["path"])
  const long = Array.from({ length: ACCESS_LIMITS.pathMax + 1 }, (_, i) => hex(String(i)))
  assert.ok(fields(validatePath(long, inventory)).includes("path"))
  assert.deepEqual(fields(validatePath([A, "", B], inventory)), ["path[1]"])
  // Repeating a node anywhere in the path is rejected at the repeat.
  assert.deepEqual(fields(validatePath([A, B, A], inventory)), ["path[2]"])
  assert.deepEqual(fields(validatePath([A, B, B, C], inventory)), ["path[2]"])
})

test("revoked, key-less and unknown references are flagged, never dropped", () => {
  assert.equal(resolveAccessNode(inventory, A).kind, "ready")
  assert.equal(resolveAccessNode(inventory, REVOKED).kind, "revoked")
  assert.equal(resolveAccessNode(inventory, KEYLESS).kind, "no_key")
  assert.equal(resolveAccessNode(inventory, hex("9")).kind, "unknown")
  assert.deepEqual(
    readyNodes(inventory).map((node) => node.node.id),
    [A, B, C, D]
  )

  const saved: MeshAccessPolicy = {
    grants: [
      { id: G1, path: [A, REVOKED] },
      { id: G2, path: [KEYLESS, B] },
      { id: G3, path: [A, hex("9"), B] },
    ],
  }
  const issues = validateAccess(saved, inventory)
  assert.deepEqual(
    issues.map((issue) => [issue.grantId, issue.field]),
    [
      [G1, "grants[0].path[1]"],
      [G2, "grants[1].path[0]"],
      [G3, "grants[2].path[1]"],
    ]
  )
  // The form built from a broken grant keeps the broken reference.
  assert.deepEqual(formPath(toGrantForm(at(saved.grants, 0))), [A, REVOKED])
})

test("policy validation catches duplicate paths, bad IDs and the grant cap", () => {
  const duplicate: MeshAccessPolicy = {
    grants: [
      { id: G1, path: [A, B] },
      { id: G2, path: [A, B] },
    ],
  }
  assert.deepEqual(fields(validateAccess(duplicate, inventory)), ["grants[1].path"])

  // The reverse path is a different grant, not a duplicate.
  assert.deepEqual(validateAccess({ grants: [{ id: G1, path: [A, B] }, { id: G2, path: [B, A] }] }, inventory), [])

  // IDs written by other clients only need the contract shape.
  assert.deepEqual(validateAccess({ grants: [{ id: "grant-1", path: [A, B] }] }, inventory), [])
  assert.deepEqual(fields(validateAccess({ grants: [{ id: "bad id", path: [A, B] }] }, inventory)), ["grants[0].id"])
  assert.deepEqual(
    fields(validateAccess({ grants: [{ id: G1, path: [A, B] }, { id: G1, path: [B, A] }] }, inventory)),
    ["grants[1].id"]
  )

  const many = Array.from({ length: ACCESS_LIMITS.grants + 1 }, (_, i) => ({
    id: i.toString(16).padStart(64, "0"),
    path: [A, B],
  }))
  assert.ok(fields(validateAccess({ grants: many }, inventory)).includes("grants"))
})

test("the reverse grant is only added when explicitly requested", () => {
  const form = { ...newGrantForm(G1), source: A, via: [B], destination: C }
  assert.equal(form.addReverse, false, "reverse defaults to off")
  assert.deepEqual(grantsFromForm(form, []), [{ id: G1, path: [A, B, C] }])

  const both = grantsFromForm({ ...form, addReverse: true }, [], () => G2)
  assert.deepEqual(both, [
    { id: G1, path: [A, B, C] },
    { id: G2, path: [C, B, A] },
  ])

  // Already granted in reverse: nothing extra, and no duplicate is created.
  const existing = [{ id: G3, path: [C, B, A] }]
  assert.deepEqual(grantsFromForm({ ...form, addReverse: true }, existing, () => G2), [
    { id: G1, path: [A, B, C] },
  ])

  // A forward grant never implies its reverse.
  assert.equal(hasReverse({ id: G1, path: [A, B, C] }, [{ id: G1, path: [A, B, C] }]), false)
  assert.equal(hasReverse({ id: G1, path: [A, B, C] }, existing), true)
  // A reverse with different intermediate order is not the exact reverse.
  assert.equal(hasReverse({ id: G1, path: [A, B, C, D] }, [{ id: G3, path: [D, B, C, A] }]), false)
})

test("the builder rejects duplicate paths and keeps the grant cap with the reverse", () => {
  const form = { ...newGrantForm(G2), source: A, destination: B }
  assert.deepEqual(fields(validateGrantForm(form, [{ id: G1, path: [A, B] }], inventory)), ["path"])

  const others = Array.from({ length: ACCESS_LIMITS.grants - 1 }, (_, i) => ({
    id: i.toString(16).padStart(64, "0"),
    path: [C, D],
  }))
  assert.deepEqual(validateGrantForm(form, others, inventory), [])
  assert.deepEqual(fields(validateGrantForm({ ...form, addReverse: true }, others, inventory)), ["grants"])
})

test("change tracking and keys follow ID and exact path, not object key order", () => {
  const base: MeshAccessPolicy = { grants: [{ id: G1, path: [A, B] }, { id: G2, path: [B, A] }] }
  const draft: MeshAccessPolicy = { grants: [{ path: [A, C, B], id: G1 }, { id: G3, path: [C, D] }] }
  assert.equal(grantChange(at(draft.grants, 0), base), "modified")
  assert.equal(grantChange(at(draft.grants, 1), base), "new")
  assert.equal(grantChange(at(base.grants, 0), base), "unchanged")
  assert.equal(removedGrantCount(draft, base), 1)
  assert.equal(sameAccess({ grants: [{ path: [A, B], id: G1 }] }, { grants: [{ id: G1, path: [A, B] }] }), true)
  assert.equal(sameAccess(base, { grants: [...base.grants].reverse() }), false)
})

test("only a successful save moves the CAS base; foreign edits are never adopted", () => {
  const sent: MeshAccessPolicy = { grants: [{ id: G1, path: [A, B] }] }
  const state = { base: snapshot(4, { grants: [] }), draft: sent }
  const sentKey = accessKey(sent)

  for (const status of [409, 422, 404, 500, null]) {
    const failure = accessSaveFailure(status, 4, "detail")
    assert.equal(settleSave(state, failure, sentKey), state)
  }
  assert.equal(accessSaveFailure(409, 4, "x").status, "conflict")
  assert.equal(accessSaveFailure(422, 4, "x").status, "invalid")
  // A controller without the access API is an error, never an empty policy.
  assert.deepEqual(accessSaveFailure(404, 4, "HTTP 404"), { status: "error", message: "保存失败：HTTP 404" })

  const canonical = snapshot(5, { grants: [{ id: G1, path: [A, B] }] })
  const saved = settleSave(state, { status: "saved", snapshot: canonical }, sentKey)
  assert.equal(saved.base.revision, 5)
  assert.deepEqual(saved.draft, canonical.policy)

  // Edited while the save was in flight: the base moves, the newer draft stays.
  const edited = { ...state, draft: { grants: [...sent.grants, { id: G2, path: [B, A] }] } }
  const kept = settleSave(edited, { status: "saved", snapshot: canonical }, sentKey)
  assert.equal(kept.base.revision, 5)
  assert.equal(kept.draft, edited.draft)
})
