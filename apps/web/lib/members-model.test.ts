import assert from "node:assert/strict"
import test from "node:test"

import type { AccountInvitation, AccountMember, NodeSummary } from "@meshway/control-client"

import {
  grantableRoles,
  invitationStatus,
  memberChanges,
  nodeOwners,
  ownershipView,
  resetPath,
  resetTokenFromHash,
  sortMembers,
} from "./members-model.ts"

const member = (userId: string, role: AccountMember["membership"]["role"], email = `${userId}@x.test`): AccountMember => ({
  user: { user_id: userId, email, email_verified: false },
  membership: { membership_id: `m-${userId}`, network_id: "n", role, user_id: userId },
})

const node = (id: string, revoked = false): NodeSummary => ({
  id,
  name: id,
  revoked_at: revoked ? "2026-01-01T00:00:00Z" : null,
  acknowledged_revision: 1,
  configuration_revision: 1,
  agent_version: "test",
  country: null,
  last_seen: "2026-01-01T00:00:00Z",
  registered_at: "2026-01-01T00:00:00Z",
  mesh_ipv4: "100.64.0.1",
  mesh_ipv6: "fd00::1",
})

test("an Admin may only invite and remove Members; an Owner manages every role", () => {
  const admin = { kind: "user" as const, userId: "a", role: "admin" as const }
  const owner = { kind: "user" as const, userId: "o", role: "owner" as const }
  assert.deepEqual(grantableRoles(admin), ["member"])
  assert.deepEqual(grantableRoles({ kind: "user", userId: "m", role: "member" }), [])
  assert.deepEqual(grantableRoles(owner), ["member", "admin", "owner"])
  assert.deepEqual(memberChanges(admin, member("x", "member")), ["remove"])
  assert.deepEqual(memberChanges(admin, member("y", "admin")), [])
  assert.deepEqual(memberChanges(admin, member("z", "owner")), [])
  assert.deepEqual(memberChanges(owner, member("x", "member")), ["owner", "admin", "remove"])
  assert.deepEqual(memberChanges({ kind: "bootstrap" }, member("x", "admin")), ["owner", "member", "remove"])
})

test("nobody removes themselves from the members table; an Owner may still step down", () => {
  assert.deepEqual(memberChanges({ kind: "user", userId: "o", role: "owner" }, member("o", "owner")), ["admin", "member"])
  assert.deepEqual(memberChanges({ kind: "user", userId: "a", role: "admin" }, member("a", "admin")), [])
  assert.deepEqual(memberChanges({ kind: "user", userId: "m", role: "member" }, member("m", "member")), [])
})

test("members sort by role, then email", () => {
  const sorted = sortMembers([member("b", "member"), member("c", "owner"), member("a", "member"), member("d", "admin")])
  assert.deepEqual(sorted.map((entry) => entry.user.user_id), ["c", "d", "a", "b"])
})

test("invitation status: revoked and accepted win over expiry", () => {
  const base: AccountInvitation = {
    invitation_id: "i",
    network_id: "n",
    email: "e@x.test",
    role: "member",
    issuer_user_id: null,
    expires_at: 100,
    revoked_at: null,
    consumed_at: null,
    consumed_by: null,
  }
  assert.equal(invitationStatus(base, 50), "pending")
  assert.equal(invitationStatus(base, 100), "expired")
  assert.equal(invitationStatus({ ...base, consumed_at: 60, consumed_by: "u" }, 200), "accepted")
  assert.equal(invitationStatus({ ...base, revoked_at: 70 }, 200), "revoked")
})

test("ownership separates members' devices, assignable legacy nodes and former members", () => {
  const members = [member("a", "owner")]
  const nodes = [node("n1"), node("n2"), node("n3", true), node("n4"), node("n5")]
  const ownership = [
    { node_id: "n1", network_id: "n", membership_id: "m-a" },
    { node_id: "n2", network_id: "n", membership_id: null },
    { node_id: "n3", network_id: "n", membership_id: null },
    { node_id: "n4", network_id: "n", membership_id: "m-gone" },
  ]
  const view = ownershipView(ownership, members, nodes)
  assert.deepEqual(view.byMembership.get("m-a")?.map((entry) => entry.id), ["n1"])
  // A revoked legacy node cannot be claimed; a node without an ownership row is not invented.
  assert.deepEqual(view.unassigned.map((entry) => entry.id), ["n2"])
  assert.deepEqual(view.formerMember.map((entry) => entry.id), ["n4"])
  const owners = nodeOwners(ownership, members)
  assert.deepEqual(owners.get("n1"), { kind: "member", email: "a@x.test", role: "owner" })
  assert.deepEqual(owners.get("n2"), { kind: "unassigned" })
  assert.deepEqual(owners.get("n4"), { kind: "former" })
  assert.equal(owners.get("n5"), undefined)
})

test("reset secret stays in the fragment", () => {
  const token = `mwr_${"c".repeat(64)}`
  const url = new URL(resetPath(token), "http://web.test")
  assert.equal(url.search, "")
  assert.equal(resetTokenFromHash(url.hash), token)
  assert.equal(resetTokenFromHash(`#token=mwi_${"c".repeat(64)}`), null)
  assert.throws(() => resetPath(`mws_${"c".repeat(64)}`))
})
