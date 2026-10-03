import type { NetworkSummary } from "@meshway/control-client"
import { useEffect, useMemo } from "react"

import { GlassToolbar } from "@/components/chrome/glass-toolbar"
import { LanguageSwitch, ThemeSwitch } from "@/components/chrome/preferences"
import { AccountPage } from "@/features/account/page"
import { AccountMenu } from "@/features/auth/account-menu"
import { useBootStore } from "@/features/boot/boot-store"
import { ConfigurationPage } from "@/features/configuration/page"
import { EMPTY_SCENE } from "@/features/globe/scene"
import { setGlobe } from "@/features/globe/store"
import { MembersPage } from "@/features/members/page"
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
 * the shell places the chrome and is the only writer of the globe. This
 * controller reports no nodes, so the globe shows none.
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
    setGlobe({
      framing: "network",
      scene: EMPTY_SCENE,
      selectedNodeId: null,
      insets,
      recede,
    })
  }, [insets, recede])
  useEffect(() => () => setGlobe({ hoveredNodeId: null, recede: false }), [])

  return (
    <>
      <OverviewPage />
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
