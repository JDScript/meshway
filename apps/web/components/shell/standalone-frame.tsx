import type { ReactNode } from "react"

import { BrandMark } from "@/components/shell/brand"
import { cn } from "@/lib/utils"

/** Pages outside a network: sign-in, device approval, landing errors, 404. */
export function StandaloneFrame({
  children,
  footer,
  width = "sm",
}: {
  children: ReactNode
  footer?: ReactNode
  width?: "sm" | "md"
}) {
  return (
    <main className="relative flex min-h-svh flex-col items-center justify-center overflow-hidden px-4 py-12">
      <div
        aria-hidden="true"
        className="bg-grid-dots pointer-events-none absolute inset-0 opacity-50 [mask-image:radial-gradient(ellipse_at_top,black_15%,transparent_65%)]"
      />
      <div className={cn("relative flex w-full flex-col gap-6", width === "sm" ? "max-w-sm" : "max-w-md")}>
        <div className="flex items-center justify-center gap-2.5">
          <BrandMark className="size-8" />
          <span className="text-base font-semibold tracking-tight">Meshway</span>
        </div>
        <section className="flex flex-col gap-5 rounded-xl border bg-card p-6 shadow-sm sm:p-7">{children}</section>
        {footer ? <div className="flex flex-col gap-2 text-center text-xs text-muted-foreground">{footer}</div> : null}
      </div>
    </main>
  )
}
