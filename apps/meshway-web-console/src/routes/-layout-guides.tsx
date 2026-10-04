import { useEffect, useState } from "react"

import {
  BAR_HEIGHT,
  DOCK_WIDTH,
  EDGE,
  MOBILE_EDGE,
  MOBILE_PANEL_TOP,
  MOBILE_SHEET_BOTTOM,
  PAGE_LEFT,
  PAGE_WIDTH,
  PANEL_TOP,
  WORKSPACE_LEFT,
} from "@/components/surface/layout"
import { useIsMobile } from "@/hooks/use-media-query"
import { useViewportHeight, useViewportWidth } from "@/hooks/use-viewport"

// Development only: the layout's lines drawn over the interface, with a
// faint 4px grid, for checking alignment by eye or in screenshots. Toggle
// with Alt+G (kept in localStorage as `meshway.guides`).

const KEY = "meshway.guides"

type Guide = { at: number; label: string; tone: "edge" | "panel" }

export function LayoutGuides() {
  const [on, setOn] = useState(() => localStorage.getItem(KEY) === "1")
  const mobile = useIsMobile()
  const width = useViewportWidth()
  const height = useViewportHeight()

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.altKey || event.code !== "KeyG") return
      setOn((value) => {
        localStorage.setItem(KEY, value ? "0" : "1")
        return !value
      })
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [])

  if (!on) return null
  const vertical: Guide[] = mobile
    ? [
        { at: MOBILE_EDGE, label: "edge", tone: "edge" },
        { at: width / 2, label: "centre", tone: "panel" },
        { at: width - MOBILE_EDGE, label: "edge", tone: "edge" },
      ]
    : [
        { at: EDGE, label: "edge", tone: "edge" },
        { at: EDGE + DOCK_WIDTH, label: "dock", tone: "panel" },
        { at: PAGE_LEFT, label: "page", tone: "panel" },
        { at: PAGE_LEFT + PAGE_WIDTH, label: "page end", tone: "panel" },
        { at: WORKSPACE_LEFT, label: "workspace", tone: "panel" },
        { at: width - EDGE, label: "edge", tone: "edge" },
      ]
  const horizontal: Guide[] = mobile
    ? [
        { at: MOBILE_EDGE, label: "bar", tone: "panel" },
        { at: MOBILE_EDGE + BAR_HEIGHT / 2, label: "bar mid", tone: "edge" },
        { at: MOBILE_EDGE + BAR_HEIGHT, label: "bar end", tone: "panel" },
        { at: MOBILE_PANEL_TOP, label: "panel", tone: "panel" },
        { at: height - MOBILE_SHEET_BOTTOM, label: "sheet end", tone: "panel" },
        {
          at: height - MOBILE_EDGE - BAR_HEIGHT,
          label: "bottom bar",
          tone: "panel",
        },
        {
          at: height - MOBILE_EDGE - BAR_HEIGHT / 2,
          label: "bottom bar mid",
          tone: "edge",
        },
        { at: height - MOBILE_EDGE, label: "edge", tone: "edge" },
      ]
    : [
        { at: EDGE, label: "edge", tone: "edge" },
        { at: EDGE + BAR_HEIGHT / 2, label: "bar mid", tone: "edge" },
        { at: EDGE + BAR_HEIGHT, label: "bar end", tone: "panel" },
        { at: PANEL_TOP, label: "panel", tone: "panel" },
        { at: height - EDGE, label: "edge", tone: "edge" },
      ]

  const colour = (tone: Guide["tone"]) =>
    tone === "edge" ? "rgb(236 72 153 / 0.8)" : "rgb(14 165 233 / 0.8)"
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none fixed inset-0 z-[1000]"
      style={{
        backgroundImage:
          "linear-gradient(to right, rgb(14 165 233 / 0.07) 1px, transparent 1px), linear-gradient(to bottom, rgb(14 165 233 / 0.07) 1px, transparent 1px)",
        backgroundSize: "4px 4px",
      }}
    >
      {vertical.map((guide) => (
        <div
          key={`v${guide.at}`}
          className="absolute inset-y-0 w-px"
          style={{ left: guide.at, background: colour(guide.tone) }}
        >
          <span
            className="absolute top-1 left-1 font-mono text-[9px] whitespace-nowrap"
            style={{ color: colour(guide.tone) }}
          >
            {guide.label} {Math.round(guide.at)}
          </span>
        </div>
      ))}
      {horizontal.map((guide) => (
        <div
          key={`h${guide.at}`}
          className="absolute inset-x-0 h-px"
          style={{ top: guide.at, background: colour(guide.tone) }}
        >
          <span
            className="absolute top-0.5 right-1 font-mono text-[9px] whitespace-nowrap"
            style={{ color: colour(guide.tone) }}
          >
            {guide.label} {Math.round(guide.at)}
          </span>
        </div>
      ))}
    </div>
  )
}
