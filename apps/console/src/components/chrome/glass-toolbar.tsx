import { cn } from "cn"
import type { ComponentProps, ReactElement } from "react"

import { SharedLayoutBg } from "@/components/motion/shared-layout-bg"

/**
 * A row of icon buttons on glass. The hover pill glides between them, as in
 * the dock. Each child is one keyed element that wraps one button.
 */
export function GlassToolbar({ children, className, ...props }: { children: ReactElement[] } & Omit<ComponentProps<"div">, "children">) {
  return (
    <div className={cn("glass pointer-events-auto flex h-(--bar-height) items-center rounded-full px-1.5", className)} {...props}>
      <SharedLayoutBg inset={0} pillClassName="rounded-full bg-fill-hover" className="flex-row gap-0.5">
        {children}
      </SharedLayoutBg>
    </div>
  )
}
