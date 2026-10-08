import { BrandMark } from "@meshway/ui/brand"
import { cn } from "@meshway/ui/cn"

import { m } from "@/paraglide/messages.js"

export function Brand({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-2 text-lead font-semibold tracking-tight",
        className
      )}
    >
      <BrandMark />
      {m.app_name()}
    </span>
  )
}
