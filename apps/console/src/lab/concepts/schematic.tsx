import { XIcon } from "lucide-react"
import { useEffect, useRef } from "react"

import { onSelectedScreen } from "@/features/globe/store"
import { lookColor } from "@/features/overview/labels"
import { m } from "@/paraglide/messages.js"

import { known, type Concept, type ConceptProps } from "../concept"

const PANEL = { top: 96, right: 48, width: 340 }

const pad = (value: number) => String(value).padStart(2, "0")

/** A leader line from the selected node to its annotation, redrawn every frame. */
function LeaderLine() {
  const path = useRef<SVGPathElement>(null)
  const ring = useRef<SVGCircleElement>(null)
  useEffect(
    () =>
      onSelectedScreen((point) => {
        const line = path.current
        const circle = ring.current
        if (!line || !circle) return
        const visible = point?.visible ?? false
        line.style.opacity = visible ? "1" : "0"
        circle.style.opacity = visible ? "1" : "0"
        if (!point) return
        const anchorX = window.innerWidth - PANEL.right - PANEL.width
        const anchorY = PANEL.top + 28
        const elbowX = anchorX - 56
        line.setAttribute("d", `M ${point.x} ${point.y} L ${elbowX} ${anchorY} L ${anchorX} ${anchorY}`)
        circle.setAttribute("cx", String(point.x))
        circle.setAttribute("cy", String(point.y))
      }),
    []
  )
  return (
    <svg className="pointer-events-none fixed inset-0 size-full" aria-hidden="true">
      <path ref={path} fill="none" stroke="var(--signal)" strokeWidth="1" strokeDasharray="3 3" className="transition-opacity duration-300" />
      <circle ref={ring} r="11" fill="none" stroke="var(--signal)" strokeWidth="1" className="transition-opacity duration-300" />
    </svg>
  )
}

/**
 * Schematic: an engineering drawing. Graphite grid paper, an x-ray globe
 * with graticule and scale rings, monospace throughout, a title block in
 * the corner and annotations wired to their node by leader lines.
 */
function Hud({ networkName, nodeCount, summary, selected, onSelect }: ConceptProps) {
  const counts = known(summary)
  const cells: [string, string][] = [
    [m.lab_network(), networkName],
    [m.stat_nodes({ count: nodeCount }), pad(nodeCount)],
    [m.stat_links({ count: counts.total }), `${pad(counts.confirmed)}/${pad(counts.total)}`],
    [m.lab_revision(), "r42"],
    [m.lab_date(), new Date().toISOString().slice(0, 10)],
  ]
  return (
    <div className="lab-schematic pointer-events-none fixed inset-0">
      <div className="absolute top-8 left-10 text-xs tracking-[0.18em] uppercase">
        <span className="text-[var(--signal)]">◆</span> meshway <span className="opacity-40">//</span> {networkName}
      </div>

      <div className="absolute right-12 bottom-10 grid w-[340px] grid-cols-[110px_1fr] text-[12px]">
        {cells.map(([label, value]) => (
          <div key={label} className="contents">
            <div className="lab-cell lab-label -mt-px -ml-px px-3 py-2">{label}</div>
            <div className="lab-cell -mt-px -ml-px truncate px-3 py-2">{value}</div>
          </div>
        ))}
        <div className="lab-cell lab-label -mt-px -ml-px px-3 py-2">{m.lab_state()}</div>
        <div className="lab-cell -mt-px -ml-px flex items-center gap-3 px-3 py-2">
          {[
            [lookColor.confirmed, counts.confirmed],
            [lookColor.one_sided, counts.oneSided],
            [lookColor.degraded, counts.degraded],
            [lookColor.configured, counts.unreported],
          ].map(([color, value], index) => (
            <span key={index} className="flex items-center gap-1.5">
              <span className="size-2 rotate-45" style={{ background: String(color) }} />
              {pad(Number(value))}
            </span>
          ))}
        </div>
      </div>

      {selected ? (
        <>
          <LeaderLine />
          <aside
            key={selected.id}
            className="lab-cell node-card-enter pointer-events-auto absolute px-5 py-4 text-[12px]"
            style={{ top: PANEL.top, right: PANEL.right, width: PANEL.width }}
          >
            <button type="button" aria-label={m.close()} onClick={() => onSelect(null)} className="absolute top-3 right-3 opacity-60 hover:opacity-100">
              <XIcon className="size-4" />
            </button>
            <div className="lab-label">{m.lab_node()}</div>
            <h2 className="mt-1 text-lg tracking-wide">{selected.name}</h2>
            <div className="mt-1 text-[var(--signal)]">● {selected.vitalityText}</div>
            <dl className="mt-4 grid grid-cols-[96px_1fr] gap-y-1.5">
              {[
                ["ipv4", selected.ipv4],
                ["ipv6", selected.ipv6],
                [m.node_report(), selected.report],
                [m.node_contact(), selected.contact],
                [m.node_receipt(), selected.receipt.text],
                [m.node_version(), selected.version],
              ].map(([label, value]) => (
                <div key={label} className="contents">
                  <dt className="lab-label pt-0.5">{label}</dt>
                  <dd className="truncate">{value}</dd>
                </div>
              ))}
            </dl>
            <div className="lab-label mt-4">{m.node_links()}</div>
            <ul className="mt-1.5">
              {selected.links.map((link) => (
                <li key={link.key}>
                  <button type="button" onClick={() => onSelect(link.peerId)} className="flex w-full items-center gap-3 py-1 text-left hover:text-[var(--signal)]">
                    <span className="size-1.5 rotate-45" style={{ background: lookColor[link.look] }} />
                    <span className="flex-1">{link.peer}</span>
                    <span className="opacity-60">{link.own ?? link.lookText}</span>
                  </button>
                </li>
              ))}
            </ul>
          </aside>
        </>
      ) : null}
    </div>
  )
}

export const schematic: Concept = {
  id: "schematic",
  name: () => m.lab_schematic(),
  theme: "dark",
  panelWidth: 0,
  look: { land: "dots", landSize: 1.15, graticule: true, stars: false, guides: true, rim: 0.35, atmosphere: 0, surfaceOpacity: 0.55 },
  Hud,
}
