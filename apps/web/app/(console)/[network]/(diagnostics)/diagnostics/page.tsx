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
        description="选择源节点与目标节点，按问题检查：已保存的授权与路径、运行邻接、DNS 与底层网络。每项都标明数据来源。"
      />
      <ConnectivityWorkbench nodes={nodes} network={network} discoveries={discoveries} />
    </PageBody>
  )
}
