import { useQuery } from "@tanstack/react-query"
import { PlusIcon } from "lucide-react"
import { useMemo, useState } from "react"

import { PanelAction, PanelHeader } from "@/components/chrome/panel-header"
import {
  SegmentCount,
  SegmentedControl,
} from "@/components/chrome/segmented-control"
import { tagsQuery } from "@/features/network/queries"
import { useCanManage } from "@/features/network/role"
import { useNetwork } from "@/features/network/use-network-view"
import { PageFrame } from "@/features/shell/page-frame"
import { m } from "@/paraglide/messages.js"

import { AddNode } from "./add-node"
import { useNodeNavigation, useNodesRoute } from "./navigation"
import { NodeList } from "./node-list"
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
  const tagsByNode = useMemo(
    () =>
      new Map(
        (tags.data?.assignments ?? []).map((assignment) => [
          assignment.node_id,
          assignment.tags.map((tag) => tag.replace(/^tag:/, "")).sort(),
        ])
      ),
    [tags.data]
  )
  const [naming, setNaming] = useState(false)
  const inventoryRead = view.nodes.data !== undefined
  const tagView = route.view === "tags"
  const nodeCount = inventoryRead
    ? view.inventory.filter((node) => node.revoked_at === null).length
    : null

  const switcher = (
    <SegmentedControl<PanelView>
      label={m.nodes_view()}
      value={route.view}
      options={[
        {
          value: "nodes",
          label: (
            <SegmentCount label={m.nodes_view_nodes()} count={nodeCount} />
          ),
        },
        {
          value: "tags",
          label: (
            <SegmentCount
              label={m.nodes_view_tags()}
              count={tags.data ? tags.data.tags.length : null}
            />
          ),
        },
      ]}
      onChange={nav.setPanel}
      className="self-start"
    />
  )
  const add = (label: string, onClick: () => void) =>
    canManage ? (
      <PanelAction label={label} icon={PlusIcon} onClick={onClick} />
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
                page="nodes"
                title={m.nav_nodes()}
                count={nodeCount}
                action={add(m.nodes_tag_new(), () => setNaming(true))}
              >
                {switcher}
              </PanelHeader>
              <TagList
                networkId={networkId}
                openTag={route.tag}
                onOpen={nav.openTag}
              />
            </div>
          ) : (
            <NodeList
              tags={tagsByNode}
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
