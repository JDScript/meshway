import { PageFrame } from "@/features/shell/page-frame"
import { useNetwork } from "@/features/network/use-network-view"

import { useNodeNavigation, useNodesRoute } from "./navigation"
import { NodeList } from "./node-list"
import { NodeWorkspace } from "./node-workspace"

/** The nodes page: the list, and one node's workspace over it. */
export function NodesPage({ networkId }: { networkId: string }) {
  const view = useNetwork()
  const route = useNodesRoute()
  const nav = useNodeNavigation(networkId)
  const inventoryRead = view.nodes.data !== undefined

  return (
    <PageFrame
      networkId={networkId}
      page="nodes"
      current={route.current}
      panel={
        <NodeList
          nodes={view.inventory}
          runtimes={view.context.runtimes}
          now={view.now}
          loading={!inventoryRead && !view.nodes.isError}
          failed={!inventoryRead && view.nodes.isError}
          view={{ q: route.q, state: route.state }}
          openId={route.node}
          onView={(next) => nav.setListView(next, route.tab)}
          onOpen={(nodeId) => nav.openNode(nodeId)}
        />
      }
      workspace={
        route.node ? (
          <NodeWorkspace
            nodeId={route.node}
            tab={route.tab}
            inventoryRead={inventoryRead}
            context={view.context}
            onClose={nav.closeNode}
            onTab={(tab) =>
              route.node ? nav.setTab(route.node, tab) : undefined
            }
            onOpenNode={(nodeId) => nav.openNode(nodeId)}
          />
        ) : null
      }
      onCloseWorkspace={nav.closeNode}
    />
  )
}
