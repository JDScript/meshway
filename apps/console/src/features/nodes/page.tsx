import { useQuery } from "@tanstack/react-query"
import { PlusIcon } from "lucide-react"
import { useState } from "react"

import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Button } from "@/components/ui/button"
import { useCanManage } from "@/features/network/role"
import { useNetwork } from "@/features/network/use-network-view"
import { tagsQuery } from "@/features/network/queries"
import { PageFrame } from "@/features/shell/page-frame"
import { m } from "@/paraglide/messages.js"

import { AddNode } from "./add-node"
import { useNodeNavigation, useNodesRoute } from "./navigation"
import { NodeList, PanelHeader } from "./node-list"
import { NodeWorkspace } from "./node-workspace"
import { NewTagDialog, TagList, TagWorkspace } from "./tags"
import type { PanelView } from "./url"

/** The nodes page: the nodes or the tags, and one node, a new node or one tag over them. */
export function NodesPage({ networkId }: { networkId: string }) {
  const view = useNetwork()
  const route = useNodesRoute()
  const nav = useNodeNavigation(networkId)
  const canManage = useCanManage(networkId)
  const tags = useQuery({
    ...tagsQuery(networkId),
    enabled: route.current,
  })
  const [naming, setNaming] = useState(false)
  const inventoryRead = view.nodes.data !== undefined
  const tagView = route.view === "tags"

  const switcher = (
    <SegmentedControl<PanelView>
      label={m.nodes_view()}
      value={route.view}
      options={[
        { value: "nodes", label: m.nodes_view_nodes() },
        {
          value: "tags",
          label: (
            <>
              {m.nodes_view_tags()}
              <span className="text-muted-foreground tabular-nums">
                {tags.data ? tags.data.tags.length : ""}
              </span>
            </>
          ),
        },
      ]}
      onChange={nav.setPanel}
      className="self-start"
    />
  )
  const add = (label: string, onClick: () => void) =>
    canManage ? (
      <Button
        variant="ghost"
        size="icon-sm"
        className="rounded-full"
        aria-label={label}
        onClick={onClick}
      >
        <PlusIcon />
      </Button>
    ) : null

  return (
    <>
      <PageFrame
        networkId={networkId}
        page="nodes"
        current={route.current}
        panel={
          tagView ? (
            <div className="flex h-full min-h-0 flex-col">
              <PanelHeader
                count=""
                switcher={switcher}
                action={add(m.nodes_tag_new(), () => setNaming(true))}
              />
              <TagList
                networkId={networkId}
                openTag={route.tag}
                onOpen={nav.openTag}
              />
            </div>
          ) : (
            <NodeList
              nodes={view.inventory}
              runtimes={view.context.runtimes}
              now={view.now}
              loading={!inventoryRead && !view.nodes.isError}
              failed={!inventoryRead && view.nodes.isError}
              view={{ q: route.q, state: route.state }}
              openId={route.node}
              onView={nav.setListView}
              onOpen={(nodeId) => nav.openNode(nodeId)}
              switcher={switcher}
              action={add(m.nodes_add_title(), nav.openAdd)}
            />
          )
        }
        workspace={
          route.node ? (
            <NodeWorkspace
              networkId={networkId}
              nodeId={route.node}
              tab={route.tab}
              detail={route.detail}
              inventoryRead={inventoryRead}
              context={view.context}
              onClose={nav.closeWorkspace}
              onTab={(tab) =>
                route.node ? nav.setTab(route.node, tab) : undefined
              }
              onDetail={(detail) =>
                route.node
                  ? nav.setDetail(route.node, route.tab, detail)
                  : undefined
              }
              onOpenNode={(nodeId) => nav.openNode(nodeId)}
            />
          ) : route.add ? (
            <AddNode networkId={networkId} onClose={nav.closeWorkspace} />
          ) : route.tag ? (
            <TagWorkspace
              networkId={networkId}
              tag={route.tag}
              context={view.context}
              onClose={nav.closeWorkspace}
              onOpenNode={(nodeId) => nav.openNode(nodeId, "manage")}
            />
          ) : null
        }
        onCloseWorkspace={nav.closeWorkspace}
      />
      <NewTagDialog
        networkId={networkId}
        open={naming}
        onOpenChange={setNaming}
        onCreated={nav.openTag}
      />
    </>
  )
}
