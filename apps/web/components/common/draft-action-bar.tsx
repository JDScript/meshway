"use client"

import type { ReactNode } from "react"

/** Presentation only: the editor retains its draft, feedback and save ownership.
 * Keep children mounted through exit, including focused controls and live regions.
 */
export function DraftActionBar({
  visible,
  children,
}: {
  visible: boolean
  children: ReactNode
}) {
  return (
    <div data-slot="draft-action-bar" data-open={visible} className="draft-action-bar">
      <div className="draft-action-bar-clip">
        <div className="draft-action-bar-surface rounded-xl border bg-background/95 shadow-lg backdrop-blur supports-backdrop-filter:bg-background/80">
          {children}
        </div>
      </div>
    </div>
  )
}
