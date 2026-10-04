import type {
  ConfigurationValidationParams,
  NetworkSummary,
} from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { useCallback, useEffect } from "react"

import { useConfigurationCommit } from "@/features/configuration/commit"
import type { DraftInput } from "@/features/configuration/draft-state"
import { pointerSegments } from "@/features/configuration/jsonc"
import { PageFrame } from "@/features/shell/page-frame"
import { networkStateQuery } from "@/features/state/queries"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { failureText } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"

import { AddNode } from "./add-node"
import { useNodeNavigation, useNodesRoute } from "./navigation"
import { NodeList } from "./node-list"
import { NodeWorkspace } from "./node-workspace"
import { nodesQuery } from "./queries"
import type { NodeTab } from "./url"

/** The tab of a node's workspace a refused field is in. */
function tabOf(pointer: string): { node: string; tab: NodeTab } | null {
  const [section, node, part] = pointerSegments(pointer) ?? []
  if (section !== "nodes" || node === undefined) return null
  return { node, tab: part === "listeners" ? "transport" : "settings" }
}

/**
 * The nodes page: the network's nodes, and one node or adding one over
 * them. Every member reads it (the controller lists members their own
 * devices); adding is for those who may edit the configuration, and when
 * that changes while adding is open, it gives way to the list. A node's
 * settings edit the configuration draft, so the page carries its commit bar.
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
  const state = useQuery({
    ...networkStateQuery(networkId),
    enabled: route.current,
  })
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
  const onShowProgress = useCallback(
    (revision: number) =>
      void navigate({
        to: "/networks/$network/configuration/$view",
        params: { network: networkId, view: "history" },
        search: { revision },
      }),
    [navigate, networkId]
  )
  const commit = useConfigurationCommit({
    networkId,
    network,
    page: "nodes",
    active: route.current,
    onInvalid,
    onShowProgress,
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
    route.current && nodes.isError,
    failureText(nodes.error) ?? m.nodes_failed()
  )

  return (
    <>
      <PageFrame
        networkId={networkId}
        page="nodes"
        current={route.current}
        panel={
          <NodeList
            networkId={networkId}
            nodes={nodes}
            state={state.data}
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
            <AddNode networkId={networkId} onClose={nav.closeWorkspace} />
          ) : null
        }
        onCloseWorkspace={nav.closeWorkspace}
      />
      {commit.dialogs}
    </>
  )
}
