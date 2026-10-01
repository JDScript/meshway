import type { Metadata } from "next"

import { ConnectivityWorkbench } from "@/components/diagnostics/connectivity-workbench"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { fetchNetwork, fetchNodes } from "@/lib/controller"

export const metadata: Metadata = { title: "诊断" }

/** Retain /monitoring and its deep links as the diagnostic entry point. */
export default async function MonitoringPage() {
  const [nodes, network] = await Promise.all([fetchNodes(), fetchNetwork()])
  return (
    <PageBody>
      <PageHeader title="诊断" description="从已保存配置到节点本地观测，按问题选择检查入口。" />
      <ConnectivityWorkbench nodes={nodes} network={network} />
    </PageBody>
  )
}
