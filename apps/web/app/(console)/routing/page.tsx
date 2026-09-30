import type { Metadata } from "next"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { PolicyEditor } from "@/components/routing/policy-editor"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { controllerUrl, fetchNodes, fetchPolicy } from "@/lib/controller"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "路由策略" }

export default async function RoutingPage() {
  const [policy, nodes] = await Promise.all([fetchPolicy(), fetchNodes()])

  return (
    <PageBody width="wide">
      <PageHeader
        title="路由策略"
        description="按来源、目标域名后缀 / CIDR / 节点 / 国家匹配流量，规划有序经由节点与互联网出口。"
        actions={
          policy.ok ? (
            <span className="text-xs text-muted-foreground tabular-nums">
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
    </PageBody>
  )
}
