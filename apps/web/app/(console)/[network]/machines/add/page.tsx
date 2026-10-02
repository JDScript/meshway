import type { Metadata } from "next"
import { connection } from "next/server"

import { AddNode } from "@/components/enrollment/add-node"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { fetchConnections, fetchManagedListeners } from "@/lib/controller"
import { takenListenerIds } from "@/lib/managed-listener-model"
import { requireNetwork } from "@/lib/network-scope"
import { requireAdminToken } from "@/lib/session"

export const metadata: Metadata = { title: "添加节点" }

export default async function AddMachinePage({ params }: { params: Promise<{ network: string }> }) {
  await connection()
  await requireAdminToken()
  const { networkId, routes, network } = await requireNetwork(params)
  // Listener IDs are network-wide; the optional listener is checked against them.
  const [managed, connections] = await Promise.all([fetchManagedListeners(networkId), fetchConnections(networkId)])
  const existingListenerIds = managed.ok && connections.ok ? takenListenerIds(managed.data, connections.data) : null

  return (
    <PageBody>
      <PageHeader
        back={{ href: routes.machines(), label: "全部节点" }}
        title="添加节点"
        description={network.ok ? `加入网络「${network.data.name}」。` : undefined}
      />
      <AddNode existingListenerIds={existingListenerIds} />
    </PageBody>
  )
}
