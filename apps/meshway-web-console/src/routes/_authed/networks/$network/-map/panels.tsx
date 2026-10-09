import type { CSSProperties } from "react"

import { failureText } from "@/api/failure"
import { NodeStatusDot } from "@/components/chrome/node-status"
import {
  EDGE,
  MOBILE_EDGE,
  MOBILE_SHEET_BOTTOM,
  PANEL_TOP,
} from "@/components/surface/layout"
import { Surface } from "@/components/surface/surface"
import { useIsMobile } from "@/hooks/use-media-query"
import { useReadFailureToast } from "@/hooks/use-read-failure-toast"
import { statusLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"
import { useBooted } from "@/state/boot"
import { usePointedNodes } from "@/state/globe"
import { chooseHistoryLayer } from "@/state/history-layer"
import { useLive } from "@/state/live"

import { useNetworkScene } from "../-shell/network-scene"
import { useNodeNavigation } from "../nodes/-navigation"
import { GroupCard, NodeCard } from "./node-card"
import type { useMapSelection } from "./selection"
import { StatusCard } from "./status-card"

/** The globe and topology share one selection card and one network summary. */
export function MapPanels({
  networkId,
  page,
  selection,
}: {
  networkId: string
  page: "overview" | "topology"
  selection: ReturnType<typeof useMapSelection>
}) {
  const scene = useNetworkScene(networkId)
  const { state, nodes, nameOf } = scene
  const mobile = useIsMobile()
  const shown = useBooted()
  const live = useLive(networkId)
  const navigation = useNodeNavigation(networkId)
  const pointed = usePointedNodes()
  const pointedNode = nodes.find((node) => pointed.includes(node.id))
  const selected = nodes.find((node) => node.id === selection.node)
  const group = nodes.filter((node) => selection.group?.includes(node.id))
  const located =
    page === "topology"
      ? new Set(scene.drawnNodes.map((node) => node.id))
      : scene.located
  useReadFailureToast(
    `topology-${networkId}`,
    scene.configurationError !== null,
    failureText(scene.configurationError) ?? m.nodes_listeners_failed()
  )
  useReadFailureToast(
    `state-${networkId}`,
    state.isError,
    failureText(state.error) ?? m.nodes_state_failed()
  )
  const place: CSSProperties = mobile
    ? {
        left: MOBILE_EDGE,
        right: MOBILE_EDGE,
        bottom: MOBILE_SHEET_BOTTOM,
        maxHeight: "55dvh",
      }
    : { right: EDGE, top: PANEL_TOP, bottom: EDGE, width: 340 }
  return (
    <>
      <div
        className="pointer-events-none absolute flex flex-col justify-between gap-3"
        style={place}
      >
        {shown && (selected || group.length > 0) ? (
          <Surface
            shown
            from={mobile ? "bottom" : "right"}
            className="glass pointer-events-auto flex min-h-0 shrink-0 flex-col overflow-hidden rounded-surface"
          >
            {selected ? (
              <NodeCard
                node={selected}
                inventory={scene.inventory?.find(
                  (node) => node.id === selected.id
                )}
                links={state.data?.links ?? []}
                onOpen={() => navigation.openNode(selected.id)}
                onClose={selection.close}
                onBack={group.length ? selection.backToGroup : undefined}
              />
            ) : (
              <GroupCard
                nodes={group}
                onSelect={selection.select}
                onClose={selection.close}
              />
            )}
          </Surface>
        ) : (
          <div />
        )}
        {!mobile || (!selected && !group.length) ? (
          <Surface
            shown={shown}
            from={mobile ? "bottom" : "right"}
            className="glass pointer-events-auto flex min-h-0 flex-col overflow-hidden rounded-surface"
          >
            <StatusCard
              state={state.isError ? undefined : state.data}
              stateError={state.isError ? state.error : null}
              nodes={nodes}
              located={located}
              live={live}
              nameOf={nameOf}
              onOpenNode={selection.select}
              mobile={mobile}
              topology={page === "topology"}
              declaration={
                scene.declared && scene.configurationRevision !== undefined
                  ? {
                      topology: scene.declared,
                      revision: scene.configurationRevision,
                    }
                  : undefined
              }
              history={{
                window: scene.window,
                onChoose: (choice) => chooseHistoryLayer(networkId, choice),
                history: scene.history,
                links: scene.historyLinks,
                located,
                nameOf,
              }}
            />
          </Surface>
        ) : null}
      </div>
      {page === "overview" &&
      !selected &&
      !group.length &&
      pointedNode &&
      !mobile ? (
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
