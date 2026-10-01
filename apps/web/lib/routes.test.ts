import assert from "node:assert/strict"
import test from "node:test"

import { consoleRoutes, isNetworkId, legacyTarget, sectionSegment } from "./routes.ts"

const id = "0123456789abcdef0123456789abcdef"
const routes = consoleRoutes(id)
const node = "ab".repeat(32)

test("network IDs are exactly the controller's 32 lowercase hex characters", () => {
  assert.equal(isNetworkId(id), true)
  for (const value of ["", id.toUpperCase(), `${id}0`, "nodes", "api", "login", "../" + id.slice(3)]) {
    assert.equal(isNetworkId(value), false, value)
  }
})

test("every console path is under the network and encodes untrusted values", () => {
  assert.equal(routes.overview, `/${id}`)
  assert.equal(routes.machine(node), `/${id}/machines/${node}`)
  assert.equal(routes.machine(node, "overview"), `/${id}/machines/${node}`)
  assert.equal(routes.machine("a/b?c", "connections"), `/${id}/machines/a%2Fb%3Fc?tab=connections`)
  assert.equal(routes.machines({ q: "edge & core" }), `/${id}/machines?q=edge+%26+core`)
  assert.equal(routes.machines({ q: "" }), `/${id}/machines`)
  assert.equal(routes.access({ rule: "r#1" }), `/${id}/access?rule=r%231`)
  assert.equal(routes.dns("dns-diagnostics"), `/${id}/dns#dns-diagnostics`)
  assert.equal(routes.topology({ src: "a", dst: undefined }, "flow-check"), `/${id}/topology?src=a#flow-check`)
})

test("section segments identify the tab without trusting another network's prefix", () => {
  assert.equal(sectionSegment(`/${id}`, routes.base), "")
  assert.equal(sectionSegment(`/${id}/machines/${node}`, routes.base), "machines")
  assert.equal(sectionSegment(`/${id}x/machines`, routes.base), null)
  assert.equal(sectionSegment("/login", routes.base), null)
})

test("pre-network URLs land on their new page and keep meaningful query values", () => {
  assert.equal(legacyTarget("overview", routes), routes.overview)
  assert.equal(legacyTarget("nodes", routes, { q: "tokyo" }), `/${id}/machines?q=tokyo`)
  assert.equal(legacyTarget("node", routes, { tab: "connections" }, node), `/${id}/machines/${node}?tab=connections`)
  assert.equal(legacyTarget("node", routes, { tab: "bogus" }, node), `/${id}/machines/${node}`)
  assert.equal(legacyTarget("enrollment", routes), `/${id}/machines/add`)
  assert.equal(legacyTarget("routing", routes, { rule: ["a", "b"] }), `/${id}/routing`)
  assert.equal(legacyTarget("tags", routes, { node }), `/${id}/tags?node=${node}`)
  assert.equal(legacyTarget("connections", routes, { node }), `/${id}/machines/${node}?tab=connections`)
  assert.equal(legacyTarget("connections", routes, { node: "x".repeat(129) }), `/${id}/connections`)
  assert.equal(legacyTarget("network", routes), `/${id}/settings`)
  assert.equal(legacyTarget("monitoring", routes, { source: node }), `/${id}/diagnostics?source=${node}`)
  assert.equal(legacyTarget("topology", routes, { src: "a", domain: "x.test", junk: ["1"] }), `/${id}/topology?src=a&domain=x.test`)
})
