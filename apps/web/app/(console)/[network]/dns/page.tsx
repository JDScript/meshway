import type { Metadata } from "next"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { DnsNames } from "@/components/network/dns-names"
import { NetworkSettings } from "@/components/network/network-settings"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { controllerUrl, fetchNetwork, fetchNodes } from "@/lib/controller"
import { requireNetwork } from "@/lib/network-scope"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "DNS" }

export default async function DnsPage({ params }: { params: Promise<{ network: string }> }) {
  const { networkId } = await requireNetwork(params)
  const [network, nodes] = await Promise.all([fetchNetwork(networkId), fetchNodes(networkId)])
  return (
    <PageBody>
      <PageHeader
        title="DNS"
        description="节点名称解析：每个节点在本网络 zone 下有稳定的完整名称，可读别名在不重名时发布。这里保存的是控制意图，节点上的解析服务需显式启用。"
        meta={network.ok ? <span>读取于 {formatClock(network.fetchedAt)}</span> : null}
      />
      {network.ok ? (
        <>
          <NetworkSettings snapshot={network.data} section="dns" />
          {nodes.ok ? (
            <DnsNames network={network.data} nodes={nodes.data.nodes} />
          ) : (
            <p role="status" className="text-xs text-destructive">节点清单读取失败，无法列出节点名称：{nodes.error}</p>
          )}
        </>
      ) : (
        <ControllerUnavailable url={controllerUrl()} error={network.error} />
      )}
    </PageBody>
  )
}
