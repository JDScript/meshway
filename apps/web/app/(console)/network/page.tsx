import type { Metadata } from "next"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { NetworkSettings } from "@/components/network/network-settings"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { controllerUrl, fetchNetwork } from "@/lib/controller"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "网络设置" }

export default async function NetworkPage() {
  const network = await fetchNetwork()
  return (
    <PageBody>
      <PageHeader
        title="网络设置"
        description="当前单个网络的持久身份、地址前缀与 DNS 控制意图。"
        actions={network.ok ? <span className="text-xs text-muted-foreground tabular-nums">读取于 {formatClock(network.fetchedAt)}</span> : null}
      />
      {network.ok ? <NetworkSettings snapshot={network.data} /> : <ControllerUnavailable url={controllerUrl()} error={network.error} />}
    </PageBody>
  )
}
