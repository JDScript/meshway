import { useId, type CSSProperties } from "react"

import { failureText } from "@/api/failure"
import { ListRow, PanelList } from "@/components/chrome/list"
import { NodeStatusDot } from "@/components/chrome/node-status"
import {
  EDGE,
  MOBILE_EDGE,
  MOBILE_MAP_SHEET,
  MOBILE_SHEET_BOTTOM,
  PAGE_WIDTH,
  PANEL_TOP,
} from "@/components/surface/layout"
import { Surface } from "@/components/surface/surface"
import { useIsMobile } from "@/hooks/use-media-query"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { statusLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"
import { useBooted } from "@/state/boot"
import { usePointedNodes } from "@/state/globe"

import { useNetworkScene } from "../-shell/network-scene"
import { LinkLevel } from "./link-level"
import { NetworkLevel } from "./network-level"
import { NodeLevel } from "./node-level"
import { LevelHead } from "./panel-kit"
import { useEscapeBack, useMapSelection, type MapPage } from "./selection"

/**
 * The one panel of both map pages, beside the globe or the topology: the
 * network (its links and nodes), a node (how it reaches the others), a
 * link (its state, capability and rule). Each level comes from the one
 * above it and goes back there.
 */
export function MapPanel({
  networkId,
  page,
}: {
  networkId: string
  page: MapPage
}) {
  const scene = useNetworkScene()
  const selection = useMapSelection(networkId, page)
  useEscapeBack(selection)
  const mobile = useIsMobile()
  const shown = useBooted()
  const pointed = usePointedNodes()
  const node = scene.nodes.find((item) => item.id === selection.node)
  const group = scene.nodes.filter((item) => selection.group?.includes(item.id))
  const pointedNode = scene.nodes.find((item) => pointed.includes(item.id))
  const groupTitle = useId()

  useReadFailureToast(
    `topology-${networkId}`,
    scene.configurationError !== null,
    failureText(scene.configurationError) ?? m.nodes_listeners_failed()
  )
  useReadFailureToast(
    `state-${networkId}`,
    scene.state.isError,
    failureText(scene.state.error) ?? m.nodes_state_failed()
  )

  const level = selection.link
    ? `link:${selection.link.join("~")}`
    : node
      ? `node:${node.id}`
      : group.length > 0
        ? "group"
        : "network"
  const place: CSSProperties = mobile
    ? {
        left: MOBILE_EDGE,
        right: MOBILE_EDGE,
        bottom: MOBILE_SHEET_BOTTOM,
        maxHeight: MOBILE_MAP_SHEET,
      }
    : {
        right: EDGE,
        top: PANEL_TOP,
        width: PAGE_WIDTH,
        maxHeight: `calc(100% - ${PANEL_TOP + EDGE}px)`,
      }

  return (
    <>
      <Surface
        shown={shown}
        from={mobile ? "bottom" : "right"}
        className="glass pointer-events-auto absolute flex flex-col overflow-hidden rounded-surface"
        style={place}
        aria-label={m.map_panel()}
      >
        <div key={level} className="swap-in flex min-h-0 flex-1 flex-col">
          {selection.link ? (
            <LinkLevel
              networkId={networkId}
              ends={selection.link}
              selection={selection}
            />
          ) : node ? (
            <NodeLevel
              networkId={networkId}
              node={node}
              selection={selection}
            />
          ) : group.length > 0 ? (
            <>
              <LevelHead
                title={m.overview_group_nodes({ count: group.length })}
                titleId={groupTitle}
                onClose={selection.close}
              />
              <PanelList aria-labelledby={groupTitle}>
                {group.map((item) => (
                  <ListRow
                    key={item.id}
                    lead={<NodeStatusDot status={item.status} />}
                    title={<span className="truncate">{item.name}</span>}
                    trail={statusLabel[item.status]()}
                    onClick={() => selection.selectNode(item.id)}
                  />
                ))}
              </PanelList>
            </>
          ) : (
            <NetworkLevel
              networkId={networkId}
              page={page}
              selection={selection}
            />
          )}
        </div>
      </Surface>
      {page === "overview" && level === "network" && pointedNode && !mobile ? (
        <div className="glass pointer-events-none absolute bottom-6 left-1/2 flex -translate-x-1/2 items-center gap-2 rounded-full px-4 py-2 text-meta">
          <NodeStatusDot status={pointedNode.status} />
          {pointed.length > 1
            ? m.overview_group_nodes({ count: pointed.length })
            : pointedNode.name}
          <span className="text-muted-foreground">
            {pointed.length === 1 ? statusLabel[pointedNode.status]() : ""}
          </span>
        </div>
      ) : null}
    </>
  )
}
