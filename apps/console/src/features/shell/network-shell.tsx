import type { NetworkSummary } from "@meshway/control-client"
import { useEffect, useMemo } from "react"

import { GlassToolbar } from "@/components/chrome/glass-toolbar"
import { LanguageSwitch, ThemeSwitch } from "@/components/chrome/preferences"
import { AccountPage } from "@/features/account/page"
import { AccountMenu } from "@/features/auth/account-menu"
import { useBootStore } from "@/features/boot/boot-store"
import { ConfigurationPage } from "@/features/configuration/page"
import { setGlobe } from "@/features/globe/store"
import { LiveIndicator } from "@/features/live/live-indicator"
import { MembersPage } from "@/features/members/page"
import { NodesPage } from "@/features/nodes/page"
import { NetworkChip } from "@/features/overview/network-chip"
import { OverviewPage } from "@/features/overview/page"
import { SettingsPage } from "@/features/settings/page"
import { useIsMobile } from "@/hooks/use-mobile"
import { useViewportHeight } from "@/hooks/use-viewport-height"
import { cn } from "@/lib/utils"

import { Dock, DockLauncher } from "./dock"
import { CHROME } from "./glass-drawer"
import { globeLayout } from "./layout"
import {
  dockFor,
  PAGES,
  useCurrentPage,
  useSwitchNetwork,
  useWorkspaceOpen,
} from "./pages"

/**
 * Everything around the globe on a network's pages, mounted once for all of
 * them: changing pages only brings surfaces in and out, while the chrome
 * stays put. Each page draws its own surfaces (`features/<page>/page.tsx`);
 * the shell places the chrome and frames the globe, and the overview
 * writes what it shows (the network's located nodes and links).
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
  const ready = useBootStore((store) => store.phase === "ready")
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

  return (
    <>
      <OverviewPage networkId={networkId} />
      <NodesPage networkId={networkId} network={network} />
      <MembersPage networkId={networkId} network={network} />
      <ConfigurationPage networkId={networkId} network={network} />
      <SettingsPage networkId={networkId} network={network} />
      <AccountPage networkId={networkId} />

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
