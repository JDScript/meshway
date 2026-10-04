import { useMatch } from "@tanstack/react-router"
import type { CSSProperties } from "react"

import { useBootStore } from "@/features/boot/boot-store"
import { EDGE, MOBILE_EDGE, MOBILE_SHEET_BOTTOM } from "@/features/shell/layout"
import { Surface } from "@/features/shell/surface"
import { useIsMobile } from "@/hooks/use-mobile"
import { m } from "@/paraglide/messages.js"

const OVERVIEW = "/_authed/networks/$network/overview" as const

/**
 * The overview: the globe is its index. This controller reports no node
 * locations or state, so the globe is empty and one line at the bottom says
 * so (on phones above the bottom bar). It goes when they are delivered, not
 * when dismissed.
 */
export function OverviewPage() {
  const mobile = useIsMobile()
  const ready = useBootStore((store) => store.phase === "ready")
  const current =
    useMatch({ from: OVERVIEW, shouldThrow: false, select: () => true }) ??
    false

  const place: CSSProperties = mobile
    ? { left: MOBILE_EDGE, right: MOBILE_EDGE, bottom: MOBILE_SHEET_BOTTOM }
    : { left: 0, right: 0, bottom: EDGE }

  return (
    <div
      className="pointer-events-none absolute flex justify-center"
      style={place}
    >
      <Surface
        shown={ready && current}
        from="bottom"
        className="glass pointer-events-auto flex h-(--bar-height) max-w-full items-center rounded-full px-4 max-md:w-full"
      >
        <p className="min-w-0 flex-1 truncate text-row text-muted-foreground">
          {m.overview_no_state()}
        </p>
      </Surface>
    </div>
  )
}
