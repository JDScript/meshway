import { XIcon } from "lucide-react"

import type { LinkLook } from "@/features/globe/scene"
import { lookColor, lookLabel } from "@/features/overview/labels"
import { m } from "@/paraglide/messages.js"

import { known, type Concept, type ConceptProps } from "../concept"

const dash: Record<LinkLook, string | undefined> = {
  confirmed: undefined,
  one_sided: undefined,
  degraded: "5 3",
  configured: "1.5 3",
}

function LineSample({ look }: { look: LinkLook }) {
  return (
    <svg width="34" height="8" aria-hidden="true" className="shrink-0">
      <line x1="1" y1="4" x2="33" y2="4" stroke={lookColor[look]} strokeWidth="1.4" strokeDasharray={dash[look]} strokeLinecap="round" />
      {look === "confirmed" ? <circle cx="22" cy="4" r="2.4" fill={lookColor[look]} /> : null}
    </svg>
  )
}

function Compass() {
  return (
    <svg viewBox="0 0 64 64" className="size-16" aria-hidden="true">
      <circle cx="32" cy="32" r="20" fill="none" stroke="currentColor" strokeOpacity="0.35" />
      <circle cx="32" cy="32" r="14" fill="none" stroke="currentColor" strokeOpacity="0.2" strokeDasharray="1 3" />
      <path d="M32 8 36 32 32 56 28 32Z" fill="none" stroke="currentColor" strokeOpacity="0.7" />
      <path d="M32 8 36 32H28Z" fill="var(--signal)" />
      <text x="32" y="6" textAnchor="middle" fontSize="7" fill="currentColor" className="lab-serif">
        {m.lab_north()}
      </text>
    </svg>
  )
}

/**
 * Cartograph: the network as a printed technical map. Warm paper with
 * grain, a ruled frame with degree ticks, a graticule on the globe, serif
 * titling, a legend that doubles as the summary, ink-red for live links.
 */
function Hud({ networkName, nodeCount, summary, selected, onSelect }: ConceptProps) {
  const counts = known(summary)
  const rows: { look: LinkLook; value: number }[] = [
    { look: "confirmed", value: counts.confirmed },
    { look: "one_sided", value: counts.oneSided },
    { look: "degraded", value: counts.degraded },
    { look: "configured", value: counts.unreported },
  ]
  return (
    <div className="lab-cartograph pointer-events-none fixed inset-0">
      <div className="lab-frame" />

      <div className="absolute top-12 left-14">
        <h1 className="lab-serif text-[38px] leading-none tracking-tight">{networkName}</h1>
        <p className="lab-serif mt-3 text-[15px] text-[var(--muted-foreground)] italic">
          {m.overview_title_line({ nodes: String(nodeCount), confirmed: String(counts.confirmed), total: String(counts.total) })}
        </p>
      </div>

      <div className="lab-paper pointer-events-auto absolute bottom-12 left-14 w-[260px] px-5 py-4">
        <div className="lab-serif text-[13px] tracking-[0.2em] uppercase">{m.lab_legend()}</div>
        <ul className="mt-3 flex flex-col gap-2.5 text-[13px]">
          {rows.map((row) => (
            <li key={row.look} className={row.value > 0 ? "flex items-center gap-3" : "flex items-center gap-3 opacity-45"}>
              <LineSample look={row.look} />
              <span className="flex-1">{lookLabel[row.look]()}</span>
              <span className="lab-serif tabular-nums">{row.value}</span>
            </li>
          ))}
        </ul>
      </div>

      <div className="absolute right-14 bottom-12 text-[var(--foreground)]">
        <Compass />
      </div>

      {selected ? (
        <aside key={selected.id} className="lab-paper node-card-enter pointer-events-auto absolute top-12 right-14 w-[360px] px-7 py-6">
          <button type="button" aria-label={m.close()} onClick={() => onSelect(null)} className="absolute top-4 right-4 p-1.5 opacity-60 transition hover:opacity-100">
            <XIcon className="size-4" />
          </button>
          <div className="lab-serif text-[12px] tracking-[0.2em] text-[var(--muted-foreground)] uppercase">{m.lab_node()}</div>
          <h2 className="lab-serif mt-2 text-[30px] leading-none">{selected.name}</h2>
          <p className="lab-serif mt-2 text-[14px] italic">{selected.vitalityText}</p>
          <hr className="my-5 border-[var(--border)]" />
          <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2.5 text-[13px]">
            {[
              [m.node_mesh_address(), `${selected.ipv4}`],
              ["", selected.ipv6],
              [m.node_report(), selected.report],
              [m.node_contact(), selected.contact],
              [m.node_receipt(), selected.receipt.text],
              [m.node_version(), selected.version],
              [m.node_location(), selected.location],
            ].map(([label, value], index) => (
              <div key={index} className="contents">
                <dt className="text-[var(--muted-foreground)]">{label}</dt>
                <dd className="text-right font-mono text-[12.5px]">{value}</dd>
              </div>
            ))}
          </dl>
          <hr className="my-5 border-[var(--border)]" />
          <ul className="flex flex-col gap-1 text-[13px]">
            {selected.links.map((link) => (
              <li key={link.key}>
                <button type="button" onClick={() => onSelect(link.peerId)} className="flex w-full items-center gap-3 py-1.5 text-left hover:underline">
                  <LineSample look={link.look} />
                  <span className="lab-serif flex-1 italic">{link.peer}</span>
                  <span className="font-mono text-[12px] text-[var(--muted-foreground)]">{link.own ?? link.lookText}</span>
                </button>
              </li>
            ))}
          </ul>
        </aside>
      ) : null}
    </div>
  )
}

export const cartograph: Concept = {
  id: "cartograph",
  name: () => m.lab_cartograph(),
  theme: "light",
  panelWidth: 420,
  look: { land: "dots", landSize: 1.2, graticule: true, stars: false, guides: false, rim: 0.7, atmosphere: 0, surfaceOpacity: 1 },
  Hud,
}
