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
        <div className="draft-action-bar-surface rounded-lg border border-warning/35 bg-card/95 shadow-lg ring-1 ring-black/[0.03] backdrop-blur supports-backdrop-filter:bg-card/85">
          {children}
        </div>
      </div>
    </div>
  )
}
