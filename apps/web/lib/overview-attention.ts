import type { MeshAccessSnapshot, NodeSummary, PolicySnapshot } from "@meshway/control-client"

import { contactBucket, summarizeInventory } from "./node-status.ts"
import type { ConsoleRoutes } from "./routes.ts"

export type AttentionKind = "access-read" | "access" | "policy-read" | "policy" | "receipt" | "stale"

export type Attention = { kind: AttentionKind; title: string; detail: string; href: string; tone: "danger" | "warn" }

/** Only conditions the controller actually reports; an empty list is not "healthy". */
export function attentionItems(
  routes: ConsoleRoutes,
  nodes: NodeSummary[],
  policy: PolicySnapshot | null,
  access: MeshAccessSnapshot | null,
  fetchedAt: number
): Attention[] {
  const summary = summarizeInventory(nodes, fetchedAt)
  const items: Attention[] = []
  if (!access) {
    items.push({ kind: "access-read", title: "无法读取访问授权", detail: "打开访问授权查看错误并重试。", href: routes.access(), tone: "danger" })
  } else if (!access.valid) {
    items.push({
      kind: "access",
      title: "已保存的访问授权无效",
      detail: `${access.diagnostics.length} 条诊断；修复前节点之间不能访问。`,
      href: routes.access(),
      tone: "danger",
    })
  }
  if (!policy) {
    items.push({ kind: "policy-read", title: "无法读取路由", detail: "打开路由查看错误并重试。", href: routes.routing(), tone: "danger" })
  } else if (!policy.valid) {
    items.push({
      kind: "policy",
      title: "已保存的路由无效",
      detail: `${policy.diagnostics.length} 条诊断；修复前节点收到拒绝全部的配置。`,
      href: routes.routing(),
      tone: "danger",
    })
  }
  const pending = summary.receipt.behind + summary.receipt.never
  if (pending > 0) {
    items.push({
      kind: "receipt",
      title: `${pending} 个节点尚未确认当前配置`,
      detail: "长期未确认请检查 agent 是否在运行。",
      href: routes.machines(),
      tone: "warn",
    })
  }
  const stale = nodes.filter((node) => contactBucket(node, fetchedAt) === "stale").length
  if (stale > 0) {
    items.push({
      kind: "stale",
      title: `${stale} 个节点超过 1 小时没有控制面联系`,
      detail: "检查 agent 是否仍在运行，或是否需要撤销。",
      href: routes.machines(),
      tone: "warn",
    })
  }
  return items
}
