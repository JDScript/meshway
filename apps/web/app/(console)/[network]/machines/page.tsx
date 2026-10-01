import { PlusIcon } from "lucide-react"
import type { Metadata } from "next"
import Link from "next/link"

import { ControllerUnavailable, NoNodes } from "@/components/common/controller-states"
import { RefreshButton } from "@/components/common/refresh-button"
import { NodesView } from "@/components/nodes/nodes-view"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { buttonVariants } from "@/components/ui/button"
import { controllerUrl, fetchNodes, fetchTags } from "@/lib/controller"
import { requireNetwork } from "@/lib/network-scope"
import { formatClock } from "@/lib/time"

export const metadata: Metadata = { title: "节点" }

export default async function MachinesPage({
  params,
  searchParams,
}: {
  params: Promise<{ network: string }>
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const { networkId, routes } = await requireNetwork(params)
  const [nodes, tags, query] = await Promise.all([fetchNodes(networkId), fetchTags(networkId), searchParams])
  const q = typeof query.q === "string" ? query.q : ""
  const tagMap = tags.ok
    ? Object.fromEntries(tags.data.assignments.map((assignment) => [assignment.node_id, assignment.tags]))
    : null

  return (
    <PageBody>
      <PageHeader
        title="节点"
        description="加入这张网络的服务器与设备。"
        meta={nodes.ok ? <span>读取于 {formatClock(nodes.fetchedAt)}</span> : null}
        actions={
          <>
            {nodes.ok ? <RefreshButton /> : null}
            <Link href={routes.addMachine} className={buttonVariants()}>
              <PlusIcon data-icon="inline-start" />
              添加节点
            </Link>
          </>
        }
      />
      {!nodes.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
      ) : nodes.data.nodes.length === 0 ? (
        <NoNodes addHref={routes.addMachine} />
      ) : (
        <>
          {!tags.ok ? (
            <p role="status" className="text-xs text-warning">标签读取失败，列表暂不显示标签：{tags.error}</p>
          ) : null}
          <NodesView key={q} nodes={nodes.data.nodes} fetchedAt={nodes.fetchedAt} initialQuery={q} tags={tagMap} />
        </>
      )}
    </PageBody>
  )
}
