import type { Metadata } from "next"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { NetworkSettings } from "@/components/network/network-settings"
import { DnsHashForward } from "@/components/network/dns-hash-forward"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { SectionTabs } from "@/components/shell/section-tabs"
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
        title="设置"
        meta={network.ok ? <span>读取于 {formatClock(network.fetchedAt)}</span> : null}
      />
      <SectionTabs section="settings" />
      {network.ok ? (
        <NetworkSettings snapshot={network.data} section="general" />
      ) : (
        <ControllerUnavailable url={controllerUrl()} error={network.error} />
      )}
    </PageBody>
  )
}
