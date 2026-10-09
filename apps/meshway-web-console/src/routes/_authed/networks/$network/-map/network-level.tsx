import { useId, useState } from "react"

import { ListNote, ListRow, PanelList } from "@/components/chrome/list"
import { NodeStatusDot } from "@/components/chrome/node-status"
import {
  SegmentCount,
  SegmentedControl,
} from "@/components/chrome/segmented-control"
import { useNow } from "@/hooks/use-now"
import { agoTime, dateTime } from "@/lib/format"
import { placeText } from "@/lib/geo"
import type { MapConnection, MapNode } from "@/lib/network-map"
import { statusLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"
import { useGlobeAvailable } from "@/state/boot"
import { highlightOnGlobe } from "@/state/globe"
import { useLive } from "@/state/live"

import { useNetworkScene } from "../-shell/network-scene"
import { HistoryMenu } from "./history-menu"
import { unlocatedReason } from "./labels"
import { linkSummary } from "./link-text"
import { LinkSwatch } from "./panel-kit"
import type { MapPage, MapSelection } from "./selection"

type List = "links" | "nodes"

const STATUS_RANK: Record<MapConnection["status"], number> = {
  connected: 0,
  connecting: 1,
  failed: 2,
  unknown: 3,
  allowed: 4,
  history: 5,
  unavailable: 6,
}

/** Where a node is, or why it is not on the globe. */
function placeOf(node: MapNode, page: MapPage): string | null {
  if (node.geo?.kind === "found") return placeText(node.geo.geo)
  return page === "overview" ? unlocatedReason(node) : null
}

/**
 * Nothing selected: the network in one line (how current it is, its nodes
 * and connections), then its links or its nodes to choose from. The
 * topology lists its declared links, the globe what the nodes report.
 */
export function NetworkLevel({
  networkId,
  page,
  selection,
}: {
  networkId: string
  page: MapPage
  selection: MapSelection
}) {
  const scene = useNetworkScene()
  const live = useLive(networkId)
  const now = useNow(5_000)
  const globe = useGlobeAvailable()
  const listId = useId()
  const [list, setList] = useState<List>(
    page === "topology" ? "links" : "nodes"
  )
  const state = scene.state.data
  const nodes = page === "topology" ? scene.drawnNodes : scene.nodes
  const links = [...(page === "topology" ? scene.edges : scene.connections)]
    .filter((edge) => edge.status !== "unavailable" || page === "topology")
    .sort(
      (x, y) =>
        STATUS_RANK[x.status] - STATUS_RANK[y.status] ||
        scene.nameOf(x.a).localeCompare(scene.nameOf(y.a)) ||
        scene.nameOf(x.b).localeCompare(scene.nameOf(y.b))
    )
  const online = nodes.filter((node) => node.status === "current").length
  const connected = scene.connections.filter(
    (edge) => edge.status === "connected"
  ).length
  const declared = scene.declared

  return (
    <>
      <header className="flex shrink-0 flex-col gap-2 px-4 pt-3 pb-2">
        <h2 id={listId} className="sr-only">
          {list === "links" ? m.map_links() : m.overview_nodes()}
        </h2>
        <div className="flex h-8 items-center gap-2">
          <SegmentedControl<List>
            size="sm"
            label={m.map_list()}
            value={list}
            onChange={setList}
            options={[
              {
                value: "links",
                label: (
                  <SegmentCount label={m.map_links()} count={links.length} />
                ),
              },
              {
                value: "nodes",
                label: (
                  <SegmentCount
                    label={m.overview_nodes()}
                    count={nodes.length}
                  />
                ),
              },
            ]}
          />
          <span className="flex-1" />
          <HistoryMenu networkId={networkId} window={scene.window} />
        </div>
        <p className="flex h-5 min-w-0 items-center gap-1.5 text-meta text-muted-foreground tabular-nums">
          <span className="truncate">
            {m.map_summary({
              online: String(online),
              nodes: String(nodes.length),
              connected: String(connected),
            })}
          </span>
          {state ? (
            <span
              className="ml-auto shrink-0"
              title={dateTime(state.asOf) ?? undefined}
            >
              {live ? m.overview_live() : m.overview_polling()} ·{" "}
              {agoTime(state.asOf, now) ?? "—"}
            </span>
          ) : null}
        </p>
      </header>

      <PanelList aria-labelledby={listId} className="flex-initial">
        {list === "links" ? (
          links.length === 0 ? (
            <ListNote>
              {page === "topology" && declared
                ? m.overview_topology_empty()
                : m.nodes_links_none()}
            </ListNote>
          ) : (
            links.map((edge) => (
              <ListRow
                key={edge.key}
                lead={<LinkSwatch look={edge.look} />}
                title={
                  <span className="truncate">
                    {scene.nameOf(edge.a)} ↔ {scene.nameOf(edge.b)}
                  </span>
                }
                subtitle={<span className="truncate">{linkSummary(edge)}</span>}
                onClick={() => selection.selectLink(edge.a, edge.b)}
              />
            ))
          )
        ) : nodes.length === 0 ? (
          <ListNote>{m.overview_no_nodes()}</ListNote>
        ) : (
          nodes.map((node) => {
            const place = placeOf(node, page)
            return (
              <ListRow
                key={node.id}
                lead={<NodeStatusDot status={node.status} />}
                title={<span className="truncate">{node.name}</span>}
                subtitle={
                  place ? <span className="truncate">{place}</span> : undefined
                }
                trail={statusLabel[node.status]()}
                onPointerEnter={() => highlightOnGlobe(node.id)}
                onPointerLeave={() => highlightOnGlobe(null)}
                onClick={() => selection.selectNode(node.id)}
              />
            )
          })
        )}
      </PanelList>

      <Notes>
        {scene.stateBehind && state ? (
          <p>{m.map_state_behind({ revision: state.configurationRevision })}</p>
        ) : null}
        {page === "topology" &&
        declared &&
        declared.totalNodes > declared.nodeIds.length ? (
          <p>
            {m.overview_topology_window({
              shown: declared.nodeIds.length,
              total: declared.totalNodes,
            })}
          </p>
        ) : null}
        {page === "overview" && !globe ? (
          <p>{m.overview_globe_unavailable()}</p>
        ) : null}
      </Notes>
    </>
  )
}

function Notes({ children }: { children: React.ReactNode }) {
  const shown = (Array.isArray(children) ? children : [children]).some(Boolean)
  return shown ? (
    <div className="flex flex-col gap-1 border-t border-divider px-4 py-3 text-meta text-muted-foreground">
      {children}
    </div>
  ) : null
}
