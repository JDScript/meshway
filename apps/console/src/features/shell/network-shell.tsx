import { cn } from "cn"
import { useCallback, useEffect, useMemo } from "react"

import { GlassToolbar } from "@/components/chrome/glass-toolbar"
import { LanguageSwitch, ThemeSwitch } from "@/components/chrome/preferences"
import { AccountMenu } from "@/features/auth/account-menu"
import { useBootStore } from "@/features/boot/boot-store"
import { EMPTY_SCENE } from "@/features/globe/scene"
import { onGlobeSelect, setGlobe } from "@/features/globe/store"
import { useNetworkView } from "@/features/network/use-network-view"
import type { NodeContext } from "@/features/nodes/node-facts"
import { NodeList } from "@/features/nodes/node-list"
import { NodeWorkspace } from "@/features/nodes/node-workspace"
import { NetworkChip } from "@/features/overview/network-chip"
import { NodePeek } from "@/features/overview/node-peek"
import { UnplacedOrb } from "@/features/overview/unplaced-orb"
import { useIsMobile } from "@/hooks/use-mobile"
import { useReadFailureToast } from "@/lib/use-read-failure-toast"
import { useViewportHeight } from "@/lib/use-viewport-height"
import { m } from "@/paraglide/messages.js"

import { Dock, DockLauncher } from "./dock"
import {
  BAR_HEIGHT,
  EDGE,
  GAP,
  globeLayout,
  MOBILE_EDGE,
  MOBILE_PAGE_HEIGHT,
  MOBILE_PANEL_TOP,
  PAGE_LEFT,
  PAGE_WIDTH,
  PANEL_TOP,
  WORKSPACE_LEFT,
} from "./layout"
import { useShellNavigation } from "./navigation"
import { useShellRoute, type NodeListView } from "./route"
import { Surface } from "./surface"

const NO_LIST: NodeListView = { q: "", state: null }

/**
 * Every surface around the globe on a network's pages, mounted once for all
 * of them: changing pages only brings surfaces in and out, while the top bar
 * and the dock stay put. What is open comes from the URL (`useShellRoute`),
 * every move goes through `useShellNavigation`, and the globe is written
 * here alone.
 */
export function NetworkShell({ networkId }: { networkId: string }) {
  const view = useNetworkView(networkId)
  const route = useShellRoute()
  const list = route.page === "nodes" ? route.list : NO_LIST
  const nav = useShellNavigation(networkId, list)
  const mobile = useIsMobile()
  const viewportHeight = useViewportHeight()
  // Nothing opens before the first-load reveal is done.
  const ready = useBootStore((store) => store.phase === "ready")

  const peekId = route.page === "overview" ? route.peek : null
  const detailId = route.page === "nodes" ? route.detail : null
  const tab = route.page === "nodes" ? route.tab : "overview"
  const peekCandidate = peekId ? view.byId.get(peekId) : undefined
  const peekNode = ready && peekCandidate && peekCandidate.revoked_at === null ? peekCandidate : null
  const pageShown = ready && route.page === "nodes"
  const workspaceId = ready ? detailId : null

  const { insets, recede } = useMemo(
    () => globeLayout({ mobile, page: pageShown, peek: peekNode !== null, workspace: workspaceId !== null, viewportHeight }),
    [mobile, pageShown, peekNode, workspaceId, viewportHeight]
  )
  const focusId = peekNode?.id ?? workspaceId
  useEffect(() => {
    setGlobe({ framing: "network", scene: view.scene, selectedNodeId: focusId, insets, recede })
  }, [view.scene, focusId, insets, recede])
  useEffect(() => () => setGlobe({ scene: EMPTY_SCENE, selectedNodeId: null, hoveredNodeId: null, recede: false }), [])

  // A node on the globe: a peek on the overview, its workspace on the nodes page.
  const page = route.page
  const onGlobe = useCallback(
    (nodeId: string | null) => {
      if (page === "overview") {
        if (nodeId) nav.peek(nodeId, peekId !== null)
        else if (peekId) nav.closePeek()
      } else if (nodeId) nav.openNode(nodeId, detailId !== null)
    },
    [page, nav, peekId, detailId]
  )
  useEffect(() => onGlobeSelect(onGlobe), [onGlobe])

  useReadFailureToast("nodes", view.nodes.isError, m.overview_nodes_failed())

  const context: NodeContext = useMemo(
    () => ({ nodes: view.byId, observation: view.observation, runtimes: view.runtimes.data ?? null, now: view.now }),
    [view.byId, view.observation, view.runtimes.data, view.now]
  )
  const unplaced = view.scene.nodes.filter((node) => node.placement === null)
  const inventoryRead = view.nodes.data !== undefined

  return (
    <>
      <Surface
        shown={pageShown}
        from={mobile ? "bottom" : "left"}
        aria-labelledby="node-list-title"
        className="glass pointer-events-auto absolute overflow-hidden rounded-surface"
        style={
          mobile
            ? { left: MOBILE_EDGE, right: MOBILE_EDGE, bottom: MOBILE_EDGE, height: `${MOBILE_PAGE_HEIGHT * 100}svh` }
            : { left: PAGE_LEFT, top: PANEL_TOP, bottom: EDGE, width: PAGE_WIDTH }
        }
      >
        <NodeList
          nodes={view.inventory}
          runtimes={context.runtimes}
          now={view.now}
          loading={!inventoryRead && !view.nodes.isError}
          failed={!inventoryRead && view.nodes.isError}
          view={list}
          openId={workspaceId}
          onView={(next) => nav.setListView(next, detailId, tab)}
          onOpen={(nodeId) => nav.openNode(nodeId, detailId !== null)}
        />
      </Surface>

      <NodeWorkspace
        nodeId={workspaceId}
        tab={tab}
        inventoryRead={inventoryRead}
        context={context}
        className="glass pointer-events-auto absolute flex flex-col overflow-hidden rounded-sheet"
        style={
          mobile
            ? { left: MOBILE_EDGE, right: MOBILE_EDGE, top: MOBILE_PANEL_TOP, bottom: MOBILE_EDGE }
            : { left: WORKSPACE_LEFT, top: PANEL_TOP, right: EDGE, bottom: EDGE }
        }
        onClose={nav.closeNode}
        onTab={(next) => {
          if (detailId) nav.setTab(detailId, next)
        }}
        onOpenNode={(nodeId) => nav.openNode(nodeId, true)}
      />

      {mobile && page !== "overview" ? null : (
        <div className={cn("pointer-events-none absolute flex", mobile ? "inset-x-0 bottom-0 justify-center p-(--mobile-edge)" : "top-(--edge) left-(--edge)")}>
          <div className="enter max-w-full min-w-0" style={mobile ? { "--enter-from": "10px" } : undefined}>
            <NetworkChip
              networkId={networkId}
              onSwitchNetwork={(target) => nav.switchNetwork(target, page)}
              name={view.network?.name ?? null}
              nodes={view.nodeCount}
              links={view.links}
              stale={view.stale}
            />
          </div>
        </div>
      )}

      {mobile ? <DockLauncher networkId={networkId} page={page} /> : <Dock networkId={networkId} page={page} />}

      {unplaced.length > 0 ? (
        <Surface
          shown={ready && page === "overview"}
          from="bottom"
          aria-label={m.unplaced_title()}
          className="pointer-events-none absolute"
          style={mobile ? { left: MOBILE_EDGE, bottom: MOBILE_EDGE + BAR_HEIGHT + GAP } : { left: EDGE, bottom: EDGE }}
        >
          <UnplacedOrb nodes={unplaced} selectedId={peekNode?.id ?? null} onSelect={(nodeId) => nav.peek(nodeId, peekId !== null)} />
        </Surface>
      ) : null}

      <GlassToolbar
        className="enter absolute top-(--mobile-edge) right-(--mobile-edge) md:top-(--edge) md:right-(--edge)"
        style={{ "--enter-delay": "80ms" }}
      >
        <span key="language" className="flex">
          <LanguageSwitch />
        </span>
        <span key="theme" className="flex">
          <ThemeSwitch />
        </span>
        <span key="account" className="flex">
          <AccountMenu />
        </span>
      </GlassToolbar>

      <NodePeek
        node={peekNode}
        mobile={mobile}
        context={context}
        onPeek={(nodeId) => nav.peek(nodeId, true)}
        onClose={nav.closePeek}
        onOpenWorkspace={(nodeId) => nav.openNode(nodeId, false)}
      />
    </>
  )
}
