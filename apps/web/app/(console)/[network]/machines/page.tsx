import { PlusIcon } from "lucide-react"
import type { Metadata } from "next"
import Link from "next/link"

import { ControllerUnavailable, NoNodes } from "@/components/common/controller-states"
import { RefreshButton } from "@/components/common/refresh-button"
import { NodesView } from "@/components/nodes/nodes-view"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import { buttonVariants } from "@/components/ui/button"
import { SectionTabs } from "@/components/shell/section-tabs"
import { controllerUrl, fetchConnections, fetchMembers, fetchNodeOwnership, fetchNodes, fetchTags } from "@/lib/controller"
import { nodeOwners } from "@/lib/members-model"
import { carriersByNode } from "@/lib/node-status"
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
  const [nodes, tags, ownership, members, connections, query] = await Promise.all([
    fetchNodes(networkId),
    fetchTags(networkId),
    fetchNodeOwnership(networkId),
    fetchMembers(networkId),
    fetchConnections(networkId),
    searchParams,
  ])
  const q = typeof query.q === "string" ? query.q : ""
  const tagMap = tags.ok
    ? Object.fromEntries(tags.data.assignments.map((assignment) => [assignment.node_id, assignment.tags]))
    : null
  const owners = ownership.ok && members.ok ? Object.fromEntries(nodeOwners(ownership.data.nodes, members.data.members)) : null
  const carriers = connections.ok ? Object.fromEntries(carriersByNode(connections.data.configuration)) : null
  const readFailures = [
    !tags.ok ? `标签：${tags.error}` : null,
    !ownership.ok ? `归属：${ownership.error}` : !members.ok ? `成员：${members.error}` : null,
    !connections.ok ? `承载：${connections.error}` : null,
  ].filter((failure) => failure !== null)

  return (
    <PageBody>
      <PageHeader
        title="节点"
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
      <SectionTabs section="nodes" />
      {!nodes.ok ? (
        <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
      ) : nodes.data.nodes.length === 0 ? (
        <NoNodes addHref={routes.addMachine} />
      ) : (
        <>
          {readFailures.length > 0 ? (
            <p role="status" className="text-xs break-words text-warning">部分信息读取失败，对应列显示为“—”。{readFailures.join("；")}</p>
          ) : null}
          <NodesView key={q} nodes={nodes.data.nodes} fetchedAt={nodes.fetchedAt} initialQuery={q} tags={tagMap} owners={owners} carriers={carriers} />
        </>
      )}
    </PageBody>
  )
}
