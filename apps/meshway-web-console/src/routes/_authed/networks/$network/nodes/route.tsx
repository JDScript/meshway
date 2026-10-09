import type { ConfigurationValidationParams } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { createFileRoute, useNavigate } from "@tanstack/react-router"
import { useCallback, useEffect } from "react"

import { failureText } from "@/api/failure"
import { useNetworkSummary } from "@/api/queries/network"
import { nodesQuery } from "@/api/queries/nodes"
import { networkStateQuery } from "@/api/queries/state"
import { useConfigurationCommit } from "@/components/configuration/commit"
import { PageFrame } from "@/components/surface/page-frame"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import type { DraftInput } from "@/lib/configuration/draft-state"
import { pointerSegments } from "@/lib/configuration/jsonc"
import { m } from "@/paraglide/messages.js"

import { useNodeNavigation, useNodesRoute } from "./-navigation"
import { NodeList } from "./-node-list"
import { zNodesSearch, type NodeTab } from "./-search"
import { NodeWorkspace } from "./$node/-workspace"
import { AddNode } from "./add/-workspace"

/** The network's nodes, and one node or adding one over them (its child routes). */
export const Route = createFileRoute("/_authed/networks/$network/nodes")({
  staticData: { page: "nodes" },
  validateSearch: zNodesSearch,
  component: NodesPage,
})

/** The tab of a node's workspace a refused field is in: every node field is a setting. */
function tabOf(pointer: string): { node: string; tab: NodeTab } | null {
  const [section, node] = pointerSegments(pointer) ?? []
  if (section !== "nodes" || node === undefined) return null
  return { node, tab: "settings" }
}

/**
 * The nodes page: the network's nodes, and one node or adding one over
 * them. Every member reads it (the controller lists members their own
 * devices); adding is for those who may edit the configuration, and when
 * that changes while adding is open, it gives way to the list. A node's
 * settings edit the configuration draft, so the page carries its commit bar.
 */
function NodesPage() {
  const { network: networkId } = Route.useParams()
  const network = useNetworkSummary(networkId)
  const navigate = useNavigate()
  const route = useNodesRoute()
  const nav = useNodeNavigation(networkId)
  const nodes = useQuery(nodesQuery(networkId))
  const state = useQuery(networkStateQuery(networkId))
  const canAdd = network?.actions.editConfiguration ?? false

  const { openNode } = nav
  const onInvalid = useCallback(
    (_input: DraftInput, params: ConfigurationValidationParams) => {
      const first = params.issues[0]?.path ?? params.requirements[0]?.path
      const at = first === undefined ? null : tabOf(first)
      if (at) openNode(at.node, at.tab)
    },
    [openNode]
  )
  const commit = useConfigurationCommit({
    networkId,
    network,
    onInvalid,
  })

  useEffect(() => {
    if (route.add && network && !canAdd)
      void navigate({
        to: "/networks/$network/nodes",
        params: { network: networkId },
        replace: true,
      })
  }, [route.add, network, canAdd, navigate, networkId])

  useReadFailureToast(
    "nodes",
    nodes.isError,
    failureText(nodes.error) ?? m.nodes_failed()
  )

  return (
    <>
      <PageFrame
        networkId={networkId}
        page="nodes"
        panel={
          <NodeList
            networkId={networkId}
            nodes={nodes}
            state={state.data}
            stateFailed={state.isError && state.data === undefined}
            q={route.q}
            openId={route.node}
            canAdd={canAdd}
            onQuery={nav.setQuery}
            onOpen={(nodeId) => nav.openNode(nodeId)}
            onAdd={nav.openAdd}
          />
        }
        workspace={
          route.node ? (
            <NodeWorkspace
              networkId={networkId}
              network={network}
              nodeId={route.node}
              state={state}
              tab={route.tab}
              nodes={nodes}
              onClose={nav.closeWorkspace}
              onTab={(tab) =>
                route.node ? nav.setTab(route.node, tab) : undefined
              }
              onDeleted={nav.toList}
            />
          ) : route.add && canAdd ? (
            <AddNode
              networkId={networkId}
              network={network}
              nodes={nodes.data}
              onClose={nav.closeWorkspace}
              onOpenNode={(nodeId) => nav.openNode(nodeId)}
            />
          ) : null
        }
        commit={commit.bar}
        onCloseWorkspace={nav.closeWorkspace}
      />
    </>
  )
}
