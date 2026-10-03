import { cn } from "@/lib/utils"
import type { ComponentProps, ReactNode } from "react"

import { useKeptWhile, useUnmountAfter } from "@/lib/use-kept"

/** Matches the hidden-state transition in `index.css` (`.surface`). */
const EXIT_MS = 260

type SurfaceFrom = "left" | "right" | "bottom" | "rise"

/**
 * One floating surface that comes and goes with the URL. It appears in
 * place (a fade with a short shift, never sliding across other surfaces),
 * keeps its last content while it leaves, then unmounts. Put the glass on
 * the surface itself: an opacity on a glass panel's wrapper would stop its
 * blur for the whole animation.
 */
export function Surface({
  shown,
  from,
  className,
  children,
  ...props
}: { shown: boolean; from: SurfaceFrom; children: ReactNode } & Omit<
  ComponentProps<"section">,
  "children"
>) {
  const mounted = useUnmountAfter(shown, EXIT_MS)
  const content = useKeptWhile(shown, children)

  if (!mounted) return null
  return (
    <section
      data-shown={shown}
      data-from={from}
      inert={!shown}
      className={cn("surface", className)}
      {...props}
    >
      {content}
    </section>
  )
}
