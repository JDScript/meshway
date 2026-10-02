import type { Metadata } from "next"
import { notFound } from "next/navigation"

import { ControllerUnavailable } from "@/components/common/controller-states"
import { NodeDetail, type NodeOwnerRead, type NodeTab } from "@/components/nodes/node-detail"
import { PageBody, PageHeader } from "@/components/shell/page-header"
import {
  controllerUrl,
  fetchConnectionRuntimes,
  fetchConnections,
  fetchEndpointDiscoveries,
  fetchManagedListeners,
  fetchMembers,
  fetchNetwork,
  fetchNodeOwnership,
  fetchNodes,
  fetchTags,
} from "@/lib/controller"
import { nodeOwners } from "@/lib/members-model"
import { requireNetwork } from "@/lib/network-scope"

export const metadata: Metadata = { title: "节点详情" }

function safeDecode(value: string): string {
  try {
    return decodeURIComponent(value)
  } catch {
    return value
  }
}

export default async function MachinePage({
  params,
  searchParams,
}: {
  params: Promise<{ network: string; node: string }>
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const [{ networkId, routes }, { node: encoded }, query] = await Promise.all([requireNetwork(params), params, searchParams])
  const nodeId = safeDecode(encoded)
  const [nodes, connections, runtimes, tags, network, discoveries, managed, ownership, members] = await Promise.all([
    fetchNodes(networkId),
    fetchConnections(networkId),
    fetchConnectionRuntimes(networkId),
    fetchTags(networkId),
    fetchNetwork(networkId),
    fetchEndpointDiscoveries(networkId),
    fetchManagedListeners(networkId),
    fetchNodeOwnership(networkId),
    fetchMembers(networkId),
  ])

  if (!nodes.ok) {
    return (
      <PageBody>
        <PageHeader back={{ href: routes.machines(), label: "全部节点" }} title="节点详情" />
        <ControllerUnavailable url={controllerUrl()} error={nodes.error} />
      </PageBody>
    )
  }
  // Only this network's inventory: a node ID from another network is a 404.
  const node = nodes.data.nodes.find((candidate) => candidate.id === nodeId)
  if (!node) notFound()
  const owner: NodeOwnerRead =
    ownership.ok && members.ok
      ? { ok: true, owner: nodeOwners(ownership.data.nodes, members.data.members).get(node.id) ?? null }
      : { ok: false, error: !ownership.ok ? ownership.error : !members.ok ? members.error : "" }
  const tab: NodeTab = query.tab === "connections" ? "connections" : "overview"

  return (
    <PageBody>
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
        discoveries={discoveries}
        network={network}
        owner={owner}
        managed={managed}
        tags={
          tags.ok
            ? { ok: true, tags: tags.data.assignments.find((assignment) => assignment.node_id === node.id)?.tags ?? [] }
            : { ok: false, error: tags.error }
        }
      />
    </PageBody>
  )
}
