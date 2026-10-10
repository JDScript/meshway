import assert from "node:assert/strict"
import type { ConfigurationIssue } from "@meshway/control-client"
import { test } from "vitest"

import { issueText, nodeNamer, saidOf } from "@/lib/configuration/diagnostics"

const EDGE = "3900c7b0-c10e-4772-8c30-a8a808546dc6"
const node = nodeNamer({ [EDGE]: { hostname: "sgp-edge" } })

const issue = (
  code: ConfigurationIssue["code"],
  params: Record<string, string> = {},
  path = "/routing/egress/rules/0"
): ConfigurationIssue => ({ code, params, path, message: "english" })

test("nodes are named by hostname, from an ID or a node selector; others as written", () => {
  assert.equal(node(EDGE), "sgp-edge")
  assert.equal(node(`node:${EDGE}`), "sgp-edge")
  assert.equal(node("node:typo"), "node:typo")
  assert.equal(
    issueText(issue("exit_not_enabled", { node: EDGE }), node),
    "sgp-edge 没有允许作为出口"
  )
})

test("an issue is worded by its code and parameters, never its English message", () => {
  assert.equal(
    issueText(
      issue("exit_outside_access", {
        node: EDGE,
        destination: "domain:telegram.org",
      }),
      node
    ),
    "sgp-edge 没有访问 telegram.org 的权限，不能经出口发往这里"
  )
  // Rules and assignments are numbered as the forms number them.
  assert.equal(
    issueText(issue("shadowed_rule", { by: "1" }), node),
    "前面的规则（到第 2 条为止）已覆盖这条规则的全部目的地，它不会生效"
  )
  // Built-in profiles go by their labels.
  assert.equal(
    issueText(
      issue("global_exit_requires_internet", {
        node: EDGE,
        profile: `global:${EDGE}`,
      }),
      node
    ),
    "sgp-edge 没有访问公网的权限，不能使用全局经 sgp-edge"
  )
  assert.equal(
    issueText(issue("invalid_jsonc", { line: "3", column: "7" }), node),
    "第 3 行第 7 列：JSONC 语法错误"
  )
})

test("a node's configuration over the limit says by how much", () => {
  const limit = (bytes: string) =>
    issueText(
      issue(
        "netmap_limit",
        { node: EDGE, bytes, maxBytes: String(8 * 1024 * 1024) },
        `/nodes/${EDGE}`
      ),
      node
    )
  assert.equal(
    limit(String(9.5 * 1024 * 1024)),
    "下发给 sgp-edge 的配置有 9.5 MiB，超过 8 MiB 的上限"
  )
  // Just over: rounding would make them equal, so in bytes.
  assert.match(
    limit(String(8 * 1024 * 1024 + 3)),
    /8.388.611 字节，超过 8.388.608 字节/
  )
})

test("reasons are worded when known, and left out when not", () => {
  assert.equal(
    issueText(issue("invalid_transport", { reason: "websocket_alpn" }), node),
    "传输设置无效：WebSocket 需要 HTTP/1.1 的 ALPN"
  )
  assert.equal(
    issueText(issue("invalid_transport", { reason: "new_reason" }), node),
    "传输设置无效"
  )
  // An edge a Mesh route needs, with no transport either way: its ends by hostname.
  const edge = { reason: "edge_transport_unavailable", a: EDGE, b: "c4334046" }
  assert.equal(
    issueText(
      issue(
        "invalid_transport",
        { ...edge, routingRule: "/routing/mesh/1" },
        "/topology/0/transports"
      ),
      node
    ),
    "Mesh 路由 2 要经过 sgp-edge 与 c4334046 的直接连接，但这条边两个方向都没有可用的承载"
  )
  assert.equal(
    issueText(issue("invalid_transport", edge, "/topology/0/transports"), node),
    "sgp-edge 与 c4334046 之间两个方向都没有可用的承载"
  )
  assert.equal(
    issueText(
      issue("rule_set_unavailable", { reason: "download_failed" }),
      node
    ),
    "远程规则集没有准备好：下载失败"
  )
  assert.equal(
    issueText(issue("rule_set_budget", { maxSets: "32" }), node),
    "一次最多准备 32 个远程规则集"
  )
  assert.equal(
    issueText(
      issue("rule_set_budget", { maxBytes: "8388608", maxEntries: "100000" }),
      node
    ),
    "规则集内容超过网络的上限：8 MiB、100000 条"
  )
})

test("a check's refusals are errors and its warnings are warnings", () => {
  const said = saidOf(
    {
      issues: [issue("empty_route_match")],
      requirements: [{ path: "/ruleSets/office", kind: "ruleSetContent" }],
      warnings: [issue("shadowed_rule", { by: "0" }, "/access/2")],
    },
    node
  )
  assert.deepEqual(
    said.map((entry) => [entry.path, entry.severity]),
    [
      ["/routing/egress/rules/0", "error"],
      ["/ruleSets/office", "error"],
      ["/access/2", "warning"],
    ]
  )
  assert.deepEqual(saidOf(null, node), [])
})
