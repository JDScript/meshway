import { useEffect, type ReactNode } from "react"

import { useBootStore } from "@/features/boot/boot-store"
import { useIsMobile } from "@/hooks/use-mobile"

import { GlassDrawer } from "./glass-drawer"
import { useSurfaceHistory } from "./history"
import {
  EDGE,
  MOBILE_EDGE,
  MOBILE_PAGE_HEIGHT,
  MOBILE_PANEL_TOP,
  PAGE_LEFT,
  PAGE_WIDTH,
  PANEL_TOP,
  WORKSPACE_LEFT,
} from "./layout"
import type { PageId } from "./pages"
import { Surface } from "./surface"

/** The id a page gives the heading of its panel or workspace, which names the surface. */
export const surfaceTitleId = (page: PageId, level: "panel" | "workspace") =>
  `${page}-${level}-title`

/**
 * One page's surfaces in their frames: the panel and the workspace as
 * floating surfaces on wide screens, as sheets on phones (swipe one away,
 * or tap outside it, to step back: the workspace to the panel, the panel to
 * the overview). Every page draws its surfaces through this, so they all
 * sit, move and dismiss alike. Nothing shows before the first load is done.
 */
export function PageFrame({
  networkId,
  page,
  current,
  panel,
  workspace,
  onCloseWorkspace,
}: {
  networkId: string
  page: PageId
  /** The URL is on this page. */
  current: boolean
  /** The page's index, headed by `surfaceTitleId(page, "panel")`; pages without one (the account) leave it out. */
  panel?: ReactNode
  /** What the open workspace shows, headed by `surfaceTitleId(page, "workspace")`; null when none is open. */
  workspace: ReactNode | null
  onCloseWorkspace: () => void
}) {
  const mobile = useIsMobile()
  const ready = useBootStore((store) => store.phase === "ready")
  const history = useSurfaceHistory(networkId)
  const panelShown = ready && current && panel !== undefined
  const workspaceShown = ready && current && workspace !== null

  // Escape closes the workspace on wide screens; a sheet handles its own.
  useEffect(() => {
    if (mobile || !workspaceShown) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) onCloseWorkspace()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [mobile, workspaceShown, onCloseWorkspace])

  if (mobile) {
    return (
      <>
        {panel !== undefined ? (
          <GlassDrawer
            open={panelShown && !workspaceShown}
            onDismiss={history.leavePage}
            side="bottom"
            labelledBy={surfaceTitleId(page, "panel")}
            style={{
              "--drawer-inset": `${MOBILE_EDGE}px`,
              "--drawer-radius": "var(--radius-surface)",
              height: `${MOBILE_PAGE_HEIGHT * 100}svh`,
            }}
          >
            {panel}
          </GlassDrawer>
        ) : null}
        <GlassDrawer
          open={workspaceShown}
          onDismiss={panel !== undefined ? onCloseWorkspace : history.leavePage}
          side="bottom"
          labelledBy={surfaceTitleId(page, "workspace")}
          style={{
            "--drawer-inset": `${MOBILE_EDGE}px`,
            height: `calc(100svh - ${MOBILE_PANEL_TOP + MOBILE_EDGE}px)`,
          }}
        >
          {workspace}
        </GlassDrawer>
      </>
    )
  }

  return (
    <>
      {panel !== undefined ? (
        <Surface
          shown={panelShown}
          from="left"
          aria-labelledby={surfaceTitleId(page, "panel")}
          className="glass pointer-events-auto absolute overflow-hidden rounded-surface"
          style={{
            left: PAGE_LEFT,
            top: PANEL_TOP,
            bottom: EDGE,
            width: PAGE_WIDTH,
          }}
        >
          {panel}
        </Surface>
      ) : null}
      <Surface
        shown={workspaceShown}
        from="rise"
        aria-labelledby={surfaceTitleId(page, "workspace")}
        className="glass pointer-events-auto absolute flex flex-col overflow-hidden rounded-sheet"
        style={{
          left: panel !== undefined ? WORKSPACE_LEFT : PAGE_LEFT,
          top: PANEL_TOP,
          right: EDGE,
          bottom: EDGE,
        }}
      >
        {workspace}
      </Surface>
    </>
  )
}
