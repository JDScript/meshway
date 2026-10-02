import { cn } from "cn"
import { useState, type CSSProperties, type ReactNode } from "react"

import { Drawer, DrawerContent } from "@/components/ui/drawer"

/** Marks the chrome (network card, toolbar, dock): touching it never dismisses a drawer. */
export const CHROME = { "data-shell-chrome": "" } as const
/** The chrome, and the menus and popovers it opens. */
const NOT_OUTSIDE = '[data-shell-chrome], [role="menu"], [data-slot="popover-content"]'

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
          if (!dismissOnOutside || (target instanceof Element && target.closest(NOT_OUTSIDE))) {
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
        className={cn("glass text-foreground data-ending-style:opacity-100", className)}
        style={{ "--bleed": "0px", ...style }}
      >
        {open ? children : kept}
      </DrawerContent>
    </Drawer>
  )
}
