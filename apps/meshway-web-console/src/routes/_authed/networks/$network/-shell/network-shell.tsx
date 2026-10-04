import type { NetworkSummary } from "@meshway/control-client"
import { useEffect, useMemo } from "react"

import { GlassToolbar } from "@/components/chrome/glass-toolbar"
import { LanguageSwitch, ThemeSwitch } from "@/components/chrome/preferences"
import { ConfigurationDialogs } from "@/components/configuration/review-dialog"
import { CHROME } from "@/components/surface/glass-drawer"
import { globeLayout } from "@/components/surface/layout"
import {
  dockFor,
  PAGES,
  useCurrentPage,
  useSwitchNetwork,
  useWorkspaceOpen,
} from "@/components/surface/pages"
import { useIsMobile } from "@/hooks/use-mobile"
import { useViewportHeight } from "@/hooks/use-viewport-height"
import { cn } from "@/lib/utils"
import { useBooted } from "@/state/boot"
import { setGlobe } from "@/state/globe"

import { AccountMenu } from "./account-menu"
import { Dock, DockLauncher } from "./dock"
import { LiveIndicator } from "./live-indicator"
import { NetworkChip } from "./network-chip"
import { useGlobeNetwork } from "./network-scene"

/**
 * Everything around the globe on a network's pages, mounted once for all of
 * them by the network's route: the chrome stays put while each page's route
 * draws its own surfaces. The shell places the chrome, frames the globe and
 * puts the network on it, and holds the configuration's review.
 */
export function NetworkShell({
  networkId,
  network,
}: {
  networkId: string
  /** null until read. */
  network: NetworkSummary | null
}) {
  const page = useCurrentPage()
  const workspaceOpen = useWorkspaceOpen()
  const mobile = useIsMobile()
  const viewportHeight = useViewportHeight()
  const ready = useBooted()
  const switchNetwork = useSwitchNetwork(networkId, page)
  const dock = dockFor(network?.actions ?? null)

  const { insets, recede } = useMemo(
    () =>
      globeLayout({
        mobile,
        page: ready && PAGES[page].panel,
        peek: false,
        workspace: ready && workspaceOpen,
        viewportHeight,
      }),
    [mobile, ready, page, workspaceOpen, viewportHeight]
  )
  useEffect(() => {
    setGlobe({ framing: "network", insets, recede })
  }, [insets, recede])
  useEffect(() => () => setGlobe({ hoveredNodeId: null, recede: false }), [])
  useGlobeNetwork(networkId)

  return (
    <>
      <ConfigurationDialogs networkId={networkId} />

      {/* Desktop: the card top left, the dock below it. Phones: one bottom bar on every page, the launcher where the thumb is. */}
      <div
        className={cn(
          "pointer-events-none absolute flex",
          mobile
            ? "inset-x-0 bottom-0 items-center gap-2 p-(--mobile-edge)"
            : "top-(--edge) left-(--edge)"
        )}
      >
        {mobile ? (
          <DockLauncher networkId={networkId} page={page} items={dock} />
        ) : null}
        <div
          {...CHROME}
          className="enter flex min-w-0 max-md:flex-1"
          style={mobile ? { "--enter-from": "10px" } : undefined}
        >
          <NetworkChip
            networkId={networkId}
            network={network}
            onSwitchNetwork={switchNetwork}
          />
        </div>
      </div>

      {mobile ? null : <Dock networkId={networkId} page={page} items={dock} />}

      <GlassToolbar
        {...CHROME}
        className="enter absolute top-(--mobile-edge) right-(--mobile-edge) md:top-(--edge) md:right-(--edge)"
        style={{ "--enter-delay": "80ms" }}
      >
        <span key="live" className="flex">
          <LiveIndicator networkId={networkId} />
        </span>
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
    </>
  )
}
