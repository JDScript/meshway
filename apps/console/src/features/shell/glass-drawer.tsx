import { cn } from "@/lib/utils"
import { useState, type CSSProperties, type ReactNode } from "react"

import { Drawer, DrawerContent } from "@/components/ui/drawer"

import { MOBILE_EDGE, MOBILE_SHEET_BOTTOM } from "./layout"

/** A bottom sheet floats above the phone's bottom bar and leaves through the screen's edge. */
const BOTTOM: CSSProperties = {
  "--drawer-inset": `${MOBILE_EDGE}px`,
  marginBottom: MOBILE_SHEET_BOTTOM,
  "--closed-transform": `translate3d(0, calc(100% + ${MOBILE_SHEET_BOTTOM + 2}px), 0)`,
}

/** Marks the chrome (network card, toolbar, dock): touching it never dismisses a drawer. */
export const CHROME = { "data-shell-chrome": "" } as const
/** The chrome, and the menus and popovers it opens. */
const NOT_OUTSIDE =
  '[data-shell-chrome], [role="menu"], [data-slot="popover-content"]'

/**
 * shadcn's Drawer as a floating glass sheet, not modal: the globe stays
 * usable. Swiping it away, pressing Escape or tapping outside (but not on
 * the chrome) dismisses it; the caller decides what that means in the URL.
 * The last content stays while it slides out.
 */
export function GlassDrawer({
  open,
  onDismiss,
  side,
  label,
  labelledBy,
  dismissOnOutside = true,
  className,
  style,
  children,
}: {
  open: boolean
  onDismiss: () => void
  side: "right" | "bottom"
  /** Names the drawer when it has no title of its own. */
  label?: string
  /** The id of the heading that names it. */
  labelledBy?: string
  dismissOnOutside?: boolean
  className?: string
  style?: CSSProperties
  children: ReactNode
}) {
  const [kept, setKept] = useState(children)
  if (open && children !== kept) setKept(children)

  return (
    <Drawer
      open={open}
      onOpenChange={(next, details) => {
        if (next) return
        if (details.reason === "outside-press") {
          const target = details.event.target
          if (
            !dismissOnOutside ||
            (target instanceof Element && target.closest(NOT_OUTSIDE))
          ) {
            details.cancel()
            return
          }
        }
        onDismiss()
      }}
      modal={false}
      swipeDirection={side === "bottom" ? "down" : "right"}
      showSwipeHandle={side === "bottom"}
    >
      <DrawerContent
        initialFocus={false}
        finalFocus={false}
        aria-label={label}
        aria-labelledby={labelledBy}
        className={cn(
          "glass text-foreground data-ending-style:opacity-100",
          className
        )}
        style={{
          "--bleed": "0px",
          ...(side === "bottom" ? BOTTOM : undefined),
          ...style,
        }}
      >
        {open ? children : kept}
      </DrawerContent>
    </Drawer>
  )
}
