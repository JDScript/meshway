import { zNodeId } from "@meshway/control-client/schemas"
import { createFileRoute } from "@tanstack/react-router"
import { useEffect, type CSSProperties } from "react"
import { z } from "zod"

import { NodeStatusDot } from "@/components/chrome/node-status"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import {
  EDGE,
  MOBILE_EDGE,
  MOBILE_SHEET_BOTTOM,
  PAGE_LEFT,
  PANEL_TOP,
} from "@/components/surface/layout"
import { Surface } from "@/components/surface/surface"
import { useIsMobile } from "@/hooks/use-media-query"
import { statusLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"
import { useBooted } from "@/state/boot"
import { usePointedNodes } from "@/state/globe"
import { chooseHistoryLayer } from "@/state/history-layer"
import { useLive } from "@/state/live"
import {
  chooseOverviewView,
  useOverviewView,
  type OverviewView,
} from "@/state/overview-view"

import { useNetworkScene } from "../-shell/network-scene"
import { useNodeNavigation } from "../nodes/-navigation"
import { GroupCard, NodeCard } from "./-node-card"
import { StatusCard } from "./-status-card"
import { Topology } from "./-topology"

export const Route = createFileRoute("/_authed/networks/$network/overview")({
  staticData: { page: "overview" },
  validateSearch: z.object({
    view: z.enum(["globe", "topology"]).optional().catch(undefined),
    node: zNodeId.optional().catch(undefined),
    group: z.array(zNodeId).max(1000).optional().catch(undefined),
  }),
  component: OverviewPage,
})

function OverviewPage() {
  const { network: networkId } = Route.useParams()
  const search = Route.useSearch()
  const navigate = Route.useNavigate()
  const remembered = useOverviewView()
  const view = search.view ?? remembered
  const mobile = useIsMobile()
  const shown = useBooted()
  const live = useLive(networkId)
  const scene = useNetworkScene(networkId)
  const { state, nodes, located, nameOf } = scene
  const navigation = useNodeNavigation(networkId)
  const pointed = usePointedNodes()
  const pointedNode = nodes.find((node) => pointed.includes(node.id))
  const selected = nodes.find((node) => node.id === search.node)
  const group = nodes.filter((node) => search.group?.includes(node.id))
  const close = () =>
    void navigate({
      search: (previous) => ({
        ...previous,
        node: undefined,
        group: undefined,
      }),
      replace: true,
    })
  const select = (node: string) =>
    void navigate({
      search: (previous) => ({ ...previous, node }),
      replace: true,
    })
  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented)
        void navigate({
          search: (previous) => ({
            ...previous,
            node: undefined,
            group: undefined,
          }),
          replace: true,
        })
    }
    document.addEventListener("keydown", escape)
    return () => document.removeEventListener("keydown", escape)
  }, [navigate])
  const side: CSSProperties = mobile
    ? {
        left: MOBILE_EDGE,
        right: MOBILE_EDGE,
        bottom: MOBILE_SHEET_BOTTOM,
        maxHeight: "55dvh",
      }
    : { right: EDGE, top: PANEL_TOP, bottom: EDGE, width: 340 }
  const selectView = (next: OverviewView) => {
    chooseOverviewView(next)
    void navigate({
      search: (previous) => ({ ...previous, view: next }),
      replace: true,
    })
  }
  return (
    <>
      <div
        className="pointer-events-auto absolute z-10"
        style={{
          left: mobile ? MOBILE_EDGE : PAGE_LEFT,
          top: mobile ? PANEL_TOP : PANEL_TOP,
        }}
      >
        <SegmentedControl<OverviewView>
          label={m.overview_view()}
          value={view}
          onChange={selectView}
          options={[
            { value: "globe", label: m.overview_globe() },
            { value: "topology", label: m.overview_topology() },
          ]}
          className="glass"
        />
      </div>
      {view === "topology" ? (
        <div
          className="pointer-events-auto absolute flex"
          style={{
            left: mobile ? MOBILE_EDGE : PAGE_LEFT,
            top: PANEL_TOP + 52,
            right: mobile ? MOBILE_EDGE : EDGE + 352,
            bottom: mobile ? MOBILE_SHEET_BOTTOM + 60 : EDGE,
          }}
        >
          <Topology
            nodes={nodes}
            links={state.data?.links ?? []}
            history={scene.historyLinks}
            selected={selected?.id ?? null}
            onSelect={select}
            onClear={close}
          />
        </div>
      ) : null}
      <div
        className="pointer-events-none absolute flex flex-col justify-between gap-3"
        style={side}
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
                onClose={close}
                onBack={
                  group.length > 0
                    ? () =>
                        void navigate({
                          search: (previous) => ({
                            ...previous,
                            node: undefined,
                          }),
                          replace: true,
                        })
                    : undefined
                }
              />
            ) : (
              <GroupCard nodes={group} onSelect={select} onClose={close} />
            )}
          </Surface>
        ) : (
          <div />
        )}
        {!mobile || (!selected && group.length === 0) ? (
          <Surface
            shown={shown}
            from={mobile ? "bottom" : "right"}
            className="glass pointer-events-auto flex min-h-0 flex-col overflow-hidden rounded-surface"
          >
            <StatusCard
              state={state.data}
              stateError={state.isError ? state.error : null}
              nodes={nodes}
              located={
                view === "topology"
                  ? new Set(nodes.map((node) => node.id))
                  : located
              }
              live={live}
              nameOf={nameOf}
              onOpenNode={select}
              mobile={mobile}
              topology={view === "topology"}
              history={{
                window: scene.window,
                onChoose: (choice) => chooseHistoryLayer(networkId, choice),
                history: scene.history,
                links: scene.historyLinks,
                located:
                  view === "topology"
                    ? new Set(nodes.map((node) => node.id))
                    : located,
                nameOf,
              }}
            />
          </Surface>
        ) : null}
      </div>
      {view === "globe" &&
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
