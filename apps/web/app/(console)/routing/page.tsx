import type { Metadata } from "next"

import { ControllerUnavailable } from "@/components/controller-states"
import { PageHeader } from "@/components/page-header"
import { PolicyEditor } from "@/components/routing/policy-editor"
import { controllerUrl, fetchNodes, fetchPolicy } from "@/lib/controller"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "路由策略" }

export default async function RoutingPage() {
  const [policy, nodes] = await Promise.all([fetchPolicy(), fetchNodes()])

  return (
    <>
      <PageHeader
        title="路由策略"
        description="按顺序匹配的路由规则：来源节点或国家，目标 Mesh 节点、域名后缀、CIDR 或国家，经由有序的必经节点与可选出口。这是控制面意图，节点之间的实际转发尚未实现。"
        actions={
          policy.ok ? (
            <span className="text-xs text-muted-foreground">
              读取于 {formatClock(policy.fetchedAt)}
            </span>
          ) : null
        }
      />
      {!policy.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={policy.error} />
      ) : !nodes.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
      ) : (
        <PolicyEditor snapshot={policy.data} nodes={nodes.data.nodes} />
      )}
    </>
  )
}
