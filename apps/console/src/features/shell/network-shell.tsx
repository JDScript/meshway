import { cn } from "cn"
import { useCallback, useEffect, useMemo } from "react"

import { GlassToolbar } from "@/components/chrome/glass-toolbar"
import { LanguageSwitch, ThemeSwitch } from "@/components/chrome/preferences"
import { AccountPage } from "@/features/account/page"
import { AccountMenu } from "@/features/auth/account-menu"
import { useBootStore } from "@/features/boot/boot-store"
import { EMPTY_SCENE } from "@/features/globe/scene"
import { onGlobeSelect, setGlobe } from "@/features/globe/store"
import {
  NetworkViewProvider,
  useNetworkView,
} from "@/features/network/use-network-view"
import { MembersPage } from "@/features/members/page"
import { useNodeNavigation, useNodesRoute } from "@/features/nodes/navigation"
import { NodesPage } from "@/features/nodes/page"
import {
  usePeekedNode,
  usePeekNavigation,
} from "@/features/overview/navigation"
import { NetworkChip } from "@/features/overview/network-chip"
import { OverviewPage } from "@/features/overview/page"
import { UnplacedButton } from "@/features/overview/unplaced-button"
import { SettingsPage } from "@/features/settings/page"
import { useIsMobile } from "@/hooks/use-mobile"
import { useReadFailureToast } from "@/lib/use-read-failure-toast"
import { useViewportHeight } from "@/lib/use-viewport-height"
import { m } from "@/paraglide/messages.js"

import { CommitBar } from "./commit-bar"
import { Dock, DockLauncher } from "./dock"
import { CHROME } from "./glass-drawer"
import { globeLayout } from "./layout"
import {
  PAGES,
  useCurrentPage,
  useSwitchNetwork,
  useWorkspaceOpen,
} from "./pages"

/**
 * Everything around the globe on a network's pages, mounted once for all of
 * them: changing pages only brings surfaces in and out, while the chrome
 * stays put. Each page draws its own surfaces (`features/<page>/page.tsx`);
 * the shell places the chrome and is the only writer of the globe.
 */
export function NetworkShell({ networkId }: { networkId: string }) {
  const view = useNetworkView(networkId)
  const page = useCurrentPage()
  const workspaceOpen = useWorkspaceOpen()
  const mobile = useIsMobile()
  const viewportHeight = useViewportHeight()
  const ready = useBootStore((store) => store.phase === "ready")

  const peekNode = usePeekedNode(view.byId)
  const peek = usePeekNavigation(networkId)
  const nodes = useNodeNavigation(networkId)
  const nodeWorkspace = useNodesRoute().node
  const switchNetwork = useSwitchNetwork(networkId, page)

  const { insets, recede } = useMemo(
    () =>
      globeLayout({
        mobile,
        page: ready && PAGES[page].panel,
        peek: peekNode !== null,
        workspace: ready && workspaceOpen,
        viewportHeight,
      }),
    [mobile, ready, page, peekNode, workspaceOpen, viewportHeight]
  )
  const focusId = peekNode?.id ?? (ready ? nodeWorkspace : null)
  useEffect(() => {
    setGlobe({
      framing: "network",
      scene: view.scene,
      selectedNodeId: focusId,
      insets,
      recede,
    })
  }, [view.scene, focusId, insets, recede])
  useEffect(
    () => () =>
      setGlobe({
        scene: EMPTY_SCENE,
        selectedNodeId: null,
        hoveredNodeId: null,
        recede: false,
      }),
    []
  )

  // A node picked on the globe (or among the unplaced): a peek on the overview, its workspace elsewhere.
  const openNode = useCallback(
    (nodeId: string | null) => {
      if (page === "overview") {
        if (nodeId) peek.peek(nodeId)
        else if (peekNode) peek.closePeek()
      } else if (nodeId) nodes.openNode(nodeId)
    },
    [page, peek, peekNode, nodes]
  )
  useEffect(() => onGlobeSelect(openNode), [openNode])

  useReadFailureToast("nodes", view.nodes.isError, m.overview_nodes_failed())
  const unplaced = view.scene.nodes.filter((node) => node.placement === null)

  return (
    <NetworkViewProvider value={view}>
      <OverviewPage networkId={networkId} />
      <NodesPage networkId={networkId} />
      <MembersPage networkId={networkId} />
      <SettingsPage networkId={networkId} />
      <AccountPage networkId={networkId} />

      {mobile && page !== "overview" ? null : (
        <div
          className={cn(
            "pointer-events-none absolute flex",
            mobile
              ? "inset-x-0 bottom-0 justify-center p-(--mobile-edge)"
              : "top-(--edge) left-(--edge)"
          )}
        >
          <div
            {...CHROME}
            className="enter max-w-full min-w-0"
            style={mobile ? { "--enter-from": "10px" } : undefined}
          >
            <NetworkChip
              networkId={networkId}
              onSwitchNetwork={switchNetwork}
              name={view.network?.name ?? null}
              nodes={view.nodeCount}
              links={view.links}
              stale={view.stale}
              trailing={
                unplaced.length > 0 ? (
                  <UnplacedButton
                    nodes={unplaced}
                    selectedId={focusId}
                    side={mobile ? "top" : "bottom"}
                    onSelect={openNode}
                  />
                ) : undefined
              }
            />
          </div>
        </div>
      )}

      {mobile ? (
        <DockLauncher networkId={networkId} page={page} />
      ) : (
        <Dock networkId={networkId} page={page} />
      )}

      <GlassToolbar
        {...CHROME}
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

      <CommitBar />
    </NetworkViewProvider>
  )
}
