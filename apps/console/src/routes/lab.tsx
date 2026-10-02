import { createFileRoute, notFound } from "@tanstack/react-router"
import { useCallback, useEffect, useMemo, useRef } from "react"
import { z } from "zod"

import "@/lab/lab.css"
import { ThemeSwitch } from "@/components/chrome/preferences"
import { useTheme } from "@/components/theme-provider"
import { onGlobeSelect, setGlobe } from "@/features/globe/store"
import { NODE_ID } from "@/features/network/queries"
import { linkSummary, overviewScene } from "@/features/overview/model"
import { aurora } from "@/lab/concepts/aurora"
import { cartograph } from "@/lab/concepts/cartograph"
import { observatory } from "@/lab/concepts/observatory"
import { schematic } from "@/lab/concepts/schematic"
import { nodeFacts } from "@/lab/facts"
import { labFixtures } from "@/lab/fixtures"
import { useNow } from "@/lib/use-now"
import { m } from "@/paraglide/messages.js"

const CONCEPTS = { observatory, cartograph, aurora, schematic }
const IDS = ["observatory", "cartograph", "aurora", "schematic"] as const

const zSearch = z.object({
  v: z.enum(IDS).catch("observatory"),
  node: z.string().regex(NODE_ID).optional().catch(undefined),
})

/**
 * Design lab: whole-direction concepts over sample data, for comparison
 * only. Development builds only; it never reaches a deployed console.
 */
export const Route = createFileRoute("/lab")({
  validateSearch: zSearch,
  beforeLoad: () => {
    if (!import.meta.env.DEV) throw notFound()
  },
  component: LabPage,
})

function LabPage() {
  const { v, node } = Route.useSearch()
  const navigate = Route.useNavigate()
  const concept = CONCEPTS[v]
  const { theme, setTheme } = useTheme()

  // A concept may insist on light or dark; the visitor's choice comes back on leave.
  const initialTheme = useRef(theme)
  useEffect(() => {
    if (concept.theme) setTheme(concept.theme)
  }, [concept, setTheme])
  useEffect(() => {
    const restore = initialTheme.current
    return () => setTheme(restore)
  }, [setTheme])

  useEffect(() => {
    document.documentElement.dataset.lab = v
    return () => {
      delete document.documentElement.dataset.lab
    }
  }, [v])

  const now = useNow(5_000)
  const bucket = Math.floor(now / 10_000)
  const data = useMemo(() => labFixtures(bucket * 10_000), [bucket])
  const { scene, observation } = useMemo(() => overviewScene({ ...data, now }), [data, now])
  const byId = useMemo(() => new Map(data.nodes.map((entry) => [entry.id, entry])), [data])
  const selected = node ? byId.get(node) : undefined
  const facts = selected ? nodeFacts(selected, byId, observation, data.runtimes, now) : null

  const select = useCallback(
    (next: string | null) => void navigate({ search: (search) => ({ ...search, node: next ?? undefined }), replace: true }),
    [navigate]
  )
  useEffect(() => onGlobeSelect(select), [select])
  useEffect(() => {
    if (!facts) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") select(null)
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [facts, select])

  const selectedId = facts?.id ?? null
  useEffect(() => {
    setGlobe({ framing: "network", scene, selectedNodeId: selectedId, rightInset: selectedId ? concept.panelWidth : 0, look: concept.look })
  }, [scene, selectedId, concept])

  return (
    <>
      <concept.Hud
        networkName="Meshway Demo"
        nodeCount={scene.nodes.length}
        summary={linkSummary(observation, true)}
        selected={facts}
        onSelect={select}
      />
      <div className="fixed inset-x-0 top-5 z-10 flex justify-center">
        <div className="glass flex items-center gap-1 rounded-full p-1 text-[13px]">
          <span className="px-3 text-xs text-muted-foreground">{m.lab_title()}</span>
          {IDS.map((id) => (
            <button
              key={id}
              type="button"
              aria-pressed={id === v}
              onClick={() => void navigate({ search: { v: id, node } })}
              className="rounded-full px-3.5 py-1.5 transition-colors aria-pressed:bg-foreground aria-pressed:text-background"
            >
              {CONCEPTS[id].name()}
            </button>
          ))}
          {concept.theme === null ? <ThemeSwitch /> : null}
        </div>
      </div>
    </>
  )
}
