import { cn } from "cn"
import type { Ref } from "react"

import type { LinkSummary } from "@/features/overview/model"
import { m } from "@/paraglide/messages.js"

import { OrbRing } from "./orb-ring"

/**
 * The slot the docked globe flies into. The globe itself is the live WebGL
 * globe, drawn above the interface exactly over this button (see the
 * engine's dock framing, which tracks the button every frame); the button
 * adds the link-state ring and leads back to the globe.
 */
export function Orb({
  ref,
  links,
  onClick,
  className,
}: {
  ref: Ref<HTMLButtonElement>
  links: LinkSummary | null
  onClick: () => void
  className?: string
}) {
  return (
    <button
      ref={ref}
      type="button"
      aria-label={m.console_back_to_globe()}
      onClick={onClick}
      className={cn("relative shrink-0 rounded-full outline-none transition-[width,height] duration-200 ease-linear focus-visible:ring-2 focus-visible:ring-sidebar-ring", className)}
    >
      <OrbRing links={links} />
    </button>
  )
}
