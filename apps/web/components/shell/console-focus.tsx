"use client"

import { usePathname } from "next/navigation"
import { createContext, useContext, useMemo, useState, type ReactNode } from "react"

import { NETWORK_ID } from "@/lib/routes"

type ConsoleFocus = {
  /** The overview opens as a full-screen map until the user enters management. */
  focus: boolean
  setFocus: (focus: boolean) => void
}

const FocusContext = createContext<ConsoleFocus | null>(null)

const isOverviewPath = (pathname: string) => {
  const [, first, ...rest] = pathname.replace(/\/$/, "").split("/")
  return first !== undefined && NETWORK_ID.test(first) && rest.length === 0
}

/**
 * Above the network segment, so switching networks keeps the user's mode.
 * Only a session's first landing on an overview (e.g. after sign-in) is
 * immersive; leaving the overview ends it, so management pages and returns
 * to the overview never replay the intro.
 */
export function ConsoleFocusProvider({ children }: { children: ReactNode }) {
  const pathname = usePathname()
  const [focus, setFocus] = useState(() => isOverviewPath(pathname))
  const [lastPath, setLastPath] = useState(pathname)
  if (lastPath !== pathname) {
    setLastPath(pathname)
    if (focus && !isOverviewPath(pathname)) setFocus(false)
  }
  const value = useMemo(() => ({ focus, setFocus }), [focus])
  return <FocusContext.Provider value={value}>{children}</FocusContext.Provider>
}

export function useConsoleFocus(): ConsoleFocus {
  const context = useContext(FocusContext)
  if (!context) throw new Error("useConsoleFocus outside ConsoleFocusProvider")
  return context
}

export { isOverviewPath }
