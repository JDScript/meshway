"use client"

import { useEffect } from "react"

import { useConsoleFocus } from "@/components/shell/console-focus"

/**
 * Rendered by an overview that could not draw its map: show the management
 * layout (sidebar, header, search) so the failure has a way out besides retry.
 */
export function EndImmersive() {
  const { setFocus } = useConsoleFocus()
  useEffect(() => {
    setFocus(false)
  }, [setFocus])
  return null
}
