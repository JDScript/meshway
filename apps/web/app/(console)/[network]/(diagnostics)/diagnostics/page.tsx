import type { Metadata } from "next"

import { ConnectivityWorkbench } from "@/components/diagnostics/connectivity-workbench"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { fetchEndpointDiscoveries, fetchNetwork, fetchNodes } from "@/lib/controller"
import { requireNetwork } from "@/lib/network-scope"

export const metadata: Metadata = { title: "诊断" }

export default async function DiagnosticsPage({ params }: { params: Promise<{ network: string }> }) {
  const { networkId } = await requireNetwork(params)
  const [nodes, network, discoveries] = await Promise.all([
    fetchNodes(networkId),
    fetchNetwork(networkId),
    fetchEndpointDiscoveries(networkId),
  ])
  return (
    <PageBody>
      <PageHeader
        title="诊断工作台"
        description="选择源与目标节点，按问题逐项检查；每项标明数据来源。"
      />
      <ConnectivityWorkbench nodes={nodes} network={network} discoveries={discoveries} />
    </PageBody>
  )
}
