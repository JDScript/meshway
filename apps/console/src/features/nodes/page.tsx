import type { NetworkSummary } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useEffect } from "react"

import { PageFrame } from "@/features/shell/page-frame"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { failureText } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"

import { AddNode } from "./add-node"
import { useNodeNavigation, useNodesRoute } from "./navigation"
import { NodeList } from "./node-list"
import { NodeWorkspace } from "./node-workspace"
import { nodesQuery } from "./queries"

/**
 * The nodes page: the network's nodes, and one node or adding one over
 * them. Every member reads it (the controller lists members their own
 * devices); adding is for those who may edit the configuration, and when
 * that changes while adding is open, it gives way to the list.
 */
export function NodesPage({
  networkId,
  network,
}: {
  networkId: string
  network: NetworkSummary | null
}) {
  const navigate = useNavigate()
  const route = useNodesRoute()
  const nav = useNodeNavigation(networkId)
  const nodes = useQuery({ ...nodesQuery(networkId), enabled: route.current })
  const canAdd = network?.actions.editConfiguration ?? false

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
    route.current && nodes.isError,
    failureText(nodes.error) ?? m.nodes_failed()
  )

  return (
    <PageFrame
      networkId={networkId}
      page="nodes"
      current={route.current}
      panel={
        <NodeList
          nodes={nodes}
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
            nodeId={route.node}
            tab={route.tab}
            nodes={nodes}
            onClose={nav.closeWorkspace}
            onTab={(tab) =>
              route.node ? nav.setTab(route.node, tab) : undefined
            }
            onDeleted={nav.toList}
          />
        ) : route.add && canAdd ? (
          <AddNode networkId={networkId} onClose={nav.closeWorkspace} />
        ) : null
      }
      onCloseWorkspace={nav.closeWorkspace}
    />
  )
}
