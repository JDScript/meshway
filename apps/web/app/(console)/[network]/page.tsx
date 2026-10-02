import type { Metadata } from "next"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { NetworkOverview } from "@/components/overview/network-overview"
import { EndImmersive } from "@/components/shell/end-immersive"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { controllerUrl, fetchConnectionRuntimes, fetchConnections, fetchMeshAccess, fetchNodes, fetchPolicy } from "@/lib/controller"
import { requireNetwork } from "@/lib/network-scope"
import { attentionItems } from "@/lib/overview-attention"

export const metadata: Metadata = { title: "概览" }

export default async function OverviewPage({ params }: { params: Promise<{ network: string }> }) {
  const scope = await requireNetwork(params)
  const { networkId, routes } = scope
  const [nodes, connections, runtimes, policy, access] = await Promise.all([
    fetchNodes(networkId),
    fetchConnections(networkId),
    fetchConnectionRuntimes(networkId),
    fetchPolicy(networkId),
    fetchMeshAccess(networkId),
  ])
  const network = scope.network.ok ? scope.network.data : null

  if (!nodes.ok) {
    return (
      <PageBody>
        <EndImmersive />
        <PageHeader title={network?.name ?? "概览"} />
        <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
      </PageBody>
    )
  }

  return (
    <NetworkOverview
      networkName={network?.name ?? "网络"}
      ipv4Prefix={network?.network.ipv4_prefix ?? null}
      nodes={nodes.data.nodes}
      fetchedAt={nodes.fetchedAt}
      connections={connections.ok ? { ok: true, data: connections.data } : { ok: false, error: connections.error }}
      initialRuntimes={runtimes}
      attention={attentionItems(routes, nodes.data.nodes, policy.ok ? policy.data : null, access.ok ? access.data : null, nodes.fetchedAt)}
    />
  )
}
