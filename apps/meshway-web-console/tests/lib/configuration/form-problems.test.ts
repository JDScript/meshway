import assert from "node:assert/strict"
import { test } from "vitest"

import { partProblems, within } from "@/lib/configuration/form-problems"

test("access: empty and malformed sets, at the controller's pointers", () => {
  const problems = partProblems("access", [
    { src: [], dst: ["10.0.0.0/8"], ip: ["tcp:0"] },
    { src: ["*"], dst: ["autogroup:internet"], ip: ["*"], bidirectional: true },
  ])
  assert.deepEqual(problems, [
    { path: "/access/0/src", code: "empty" },
    { path: "/access/0/ip", code: "protocol" },
    { path: "/access/1", code: "bidirectional" },
  ])
})

test("DNS: resolvers must exist, overrideLocal needs a default", () => {
  assert.deepEqual(
    partProblems("dns", {
      domain: "mesh.example",
      overrideLocal: true,
      servers: { office: { type: "udp", server: "10.0.0.53" } },
      default: [],
      routes: { "corp.example": ["missing"] },
    }),
    [
      { path: "/dns/default", code: "default" },
      { path: "/dns/routes/corp.example", code: "resolver" },
    ]
  )
  assert.deepEqual(
    partProblems("dns", {
      domain: "mesh.example",
      servers: { a: { type: "udp", server: "example.com" } },
    }),
    [{ path: "/dns/servers/a", code: "server" }]
  )
})

test("groups and owners hold emails (and groups, for owners)", () => {
  assert.deepEqual(
    partProblems("groups", { "group:a": ["x@example.com", "nobody"] }),
    [{ path: "/groups/group:a", code: "member" }]
  )
  assert.deepEqual(
    partProblems("tagsOwners", { "tag:a": ["group:a", "x@example.com"] }),
    []
  )
})

test("pools hold prefixes of their own family", () => {
  assert.deepEqual(
    partProblems("addressPools", { ipv4: ["fd42::/48"], ipv6: [] }),
    [{ path: "/addressPools/ipv4", code: "cidr" }]
  )
})

test("pointers within a prefix, segment by segment", () => {
  assert.equal(within("/access/1/src", "/access/1"), true)
  assert.equal(within("/access/1", "/access/1"), true)
  assert.equal(within("/access/10", "/access/1"), false)
  assert.equal(within("/groups/group:a~1b", "/groups/group:a~1b"), true)
})
