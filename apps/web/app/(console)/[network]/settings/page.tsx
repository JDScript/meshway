import type { Metadata } from "next"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { NetworkSettings } from "@/components/network/network-settings"
import { DnsHashForward } from "@/components/network/dns-hash-forward"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { controllerUrl, fetchNetwork } from "@/lib/controller"
import { requireNetwork } from "@/lib/network-scope"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "网络设置" }

export default async function SettingsPage({ params }: { params: Promise<{ network: string }> }) {
  const { networkId, routes } = await requireNetwork(params)
  const network = await fetchNetwork(networkId)
  return (
    <PageBody>
      {/* Old /network#dns-* links now belong to the DNS page. */}
      <DnsHashForward href={routes.dns()} />
      <PageHeader
        title="网络设置"
        description="这张网络的名称、稳定 ID 与地址前缀。成员、访问控制与 DNS 都按网络隔离。"
        meta={network.ok ? <span>读取于 {formatClock(network.fetchedAt)}</span> : null}
      />
      {network.ok ? (
        <NetworkSettings snapshot={network.data} section="general" />
      ) : (
        <ControllerUnavailable url={controllerUrl()} error={network.error} />
      )}
    </PageBody>
  )
}
