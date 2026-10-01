import type { Metadata } from "next"
import { connection } from "next/server"

import { AddNode } from "@/components/enrollment/add-node"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { requireNetwork } from "@/lib/network-scope"
import { requireAdminToken } from "@/lib/session"

export const metadata: Metadata = { title: "添加节点" }

export default async function AddMachinePage({ params }: { params: Promise<{ network: string }> }) {
  await connection()
  await requireAdminToken()
  const { routes, network } = await requireNetwork(params)

  return (
    <PageBody>
      <PageHeader
        back={{ href: routes.machines(), label: "全部节点" }}
        title="添加节点"
        description={
          network.ok
            ? `加入网络「${network.data.name}」。服务器、电脑和手机是同一种节点，路径上的职责由策略与可达性决定。`
            : "服务器、电脑和手机是同一种节点，路径上的职责由策略与可达性决定。"
        }
      />
      <AddNode />
    </PageBody>
  )
}
