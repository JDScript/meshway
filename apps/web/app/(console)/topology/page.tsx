import type { Metadata } from "next"

import { ControllerUnavailable, NoNodes } from "@/components/common/controller-states"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { TopologyView } from "@/components/topology/topology-view"
import {
  controllerUrl,
  fetchConnectionRuntimes,
  fetchConnections,
  fetchNodes,
  fetchPolicy,
} from "@/lib/controller"

export const metadata: Metadata = { title: "网络拓扑" }

export default async function TopologyPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const [policy, nodes, connections, runtimes, params] = await Promise.all([
    fetchPolicy(),
    fetchNodes(),
    fetchConnections(),
    fetchConnectionRuntimes(),
    searchParams,
  ])
  const nodeId = typeof params.node === "string" ? params.node : null

  return (
    <PageBody width="wide">
      <PageHeader
        title="网络拓扑"
        description="节点 agent 自报的已认证邻接、已保存的连接意图，以及按已保存策略编译的规划路径。"
      />
      {!policy.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={policy.error} />
      ) : !nodes.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
      ) : nodes.data.nodes.length === 0 ? (
        <NoNodes />
      ) : (
        <TopologyView
          // A new policy revision starts a fresh view: plans never outlive it.
          key={`${policy.data.revision}-${nodeId ?? ""}`}
          snapshot={policy.data}
          nodes={nodes.data.nodes}
          fetchedAt={nodes.fetchedAt}
          initialConnections={
            connections.ok
              ? { ok: true, data: connections.data, fetchedAt: connections.fetchedAt }
              : { ok: false, error: connections.error, fetchedAt: connections.fetchedAt }
          }
          initialRuntimes={runtimes}
          initialNodeId={nodeId}
        />
      )}
    </PageBody>
  )
}
