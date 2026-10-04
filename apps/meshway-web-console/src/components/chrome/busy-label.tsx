import type { ReactNode } from "react"

import { Spinner } from "@/components/ui/spinner"

/**
 * A button's label while what it started is on its way: the spinner takes
 * the label's place and the label keeps its room, so the button never
 * changes width.
 */
export function BusyLabel({
  busy,
  children,
}: {
  busy: boolean
  children: ReactNode
}) {
  return (
    <span className="relative inline-flex items-center justify-center">
      <span
        className={
          busy
            ? "invisible inline-flex items-center gap-1.5"
            : "inline-flex items-center gap-1.5"
        }
      >
        {children}
      </span>
      {busy ? <Spinner className="absolute" /> : null}
    </span>
  )
}
