import type { Metadata } from "next"

import { ControllerUnavailable, NoNodes } from "@/components/common/controller-states"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { TopologyView } from "@/components/topology/topology-view"
import { topologyInitialFlow } from "@/lib/flow-navigation"
import { controllerUrl, fetchConnectionRuntimes, fetchConnections, fetchNodes, fetchPolicy } from "@/lib/controller"
import { requireNetwork } from "@/lib/network-scope"

export const metadata: Metadata = { title: "拓扑与流量检查" }

export default async function TopologyPage({
  params,
  searchParams,
}: {
  params: Promise<{ network: string }>
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const { networkId, routes } = await requireNetwork(params)
  const [policy, nodes, connections, runtimes, query] = await Promise.all([
    fetchPolicy(networkId),
    fetchNodes(networkId),
    fetchConnections(networkId),
    fetchConnectionRuntimes(networkId),
    searchParams,
  ])
  const nodeId = typeof query.node === "string" ? query.node : null
  const initialFlow = topologyInitialFlow(query)

  return (
    <PageBody>
      <PageHeader
        title="拓扑与流量检查"
        description="Agent 上报的邻接，以及按已保存策略规划的路径。"
      />
      {!policy.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={policy.error} />
      ) : !nodes.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
      ) : nodes.data.nodes.length === 0 ? (
        <NoNodes addHref={routes.addMachine} />
      ) : (
        <TopologyView
          // A new policy revision starts a fresh view: plans never outlive it.
          key={`${policy.data.revision}-${nodeId ?? ""}-${JSON.stringify(initialFlow)}`}
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
          initialFlow={initialFlow}
        />
      )}
    </PageBody>
  )
}
