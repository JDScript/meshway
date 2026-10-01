import type { Metadata } from "next"
import { notFound } from "next/navigation"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { NodeDetail, type NodeTab } from "@/components/nodes/node-detail"
import { PageBody } from "@/components/shell/page-header"
import {
  controllerUrl,
  fetchConnectionRuntimes,
  fetchConnections,
  fetchNodes,
  fetchTags,
} from "@/lib/controller"

export const metadata: Metadata = { title: "节点详情" }

export default async function NodePage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const [{ id }, query] = await Promise.all([params, searchParams])
  const nodeId = safeDecode(id)
  const [nodes, connections, runtimes, tags] = await Promise.all([
    fetchNodes(),
    fetchConnections(),
    fetchConnectionRuntimes(),
    fetchTags(),
  ])

  if (!nodes.ok) {
    return (
      <PageBody>
        <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
      </PageBody>
    )
  }
  const node = nodes.data.nodes.find((candidate) => candidate.id === nodeId)
  if (!node) notFound()
  const tab: NodeTab = query.tab === "connections" ? "connections" : "overview"

  return (
    <PageBody width="wide">
      <NodeDetail
        node={node}
        fetchedAt={nodes.fetchedAt}
        initialTab={tab}
        connections={
          connections.ok
            ? { ok: true, data: connections.data, fetchedAt: connections.fetchedAt }
            : { ok: false, error: connections.error }
        }
        controllerUrl={controllerUrl()}
        initialRuntimes={runtimes}
        tags={
          tags.ok
            ? { ok: true, tags: tags.data.assignments.find((assignment) => assignment.node_id === node.id)?.tags ?? [] }
            : { ok: false, error: tags.error }
        }
      />
    </PageBody>
  )
}

function safeDecode(value: string): string {
  try {
    return decodeURIComponent(value)
  } catch {
    return value
  }
}
