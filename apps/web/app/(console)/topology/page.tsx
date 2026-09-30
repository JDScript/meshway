import type { Metadata } from "next"

import { ControllerUnavailable, NoNodes } from "@/components/common/controller-states"
import { RefreshButton } from "@/components/common/refresh-button"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { TopologyView } from "@/components/topology/topology-view"
import { controllerUrl, fetchNodes, fetchPolicy } from "@/lib/controller"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "逻辑拓扑" }

export default async function TopologyPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const [policy, nodes, params] = await Promise.all([fetchPolicy(), fetchNodes(), searchParams])
  const nodeId = typeof params.node === "string" ? params.node : null

  return (
    <PageBody width="wide">
      <PageHeader
        title="逻辑拓扑"
        description="节点清单与已保存路由策略编译出的规划路径。尚无实际链路与测量数据。"
        actions={
          policy.ok && nodes.ok ? (
            <>
              <span className="text-xs text-muted-foreground tabular-nums">
                读取于 {formatClock(nodes.fetchedAt)}
              </span>
              <RefreshButton />
            </>
          ) : null
        }
      />
      {!policy.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={policy.error} />
      ) : !nodes.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
      ) : nodes.data.nodes.length === 0 ? (
        <NoNodes />
      ) : (
        <TopologyView
          // A new snapshot revision starts a fresh view: plans never outlive it.
          key={`${policy.data.revision}-${nodeId ?? ""}`}
          snapshot={policy.data}
          nodes={nodes.data.nodes}
          fetchedAt={nodes.fetchedAt}
          initialNodeId={nodeId}
        />
      )}
    </PageBody>
  )
}
