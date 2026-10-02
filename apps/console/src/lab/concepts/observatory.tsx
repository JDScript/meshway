import { XIcon } from "lucide-react"

import { lookColor, vitalityColor } from "@/features/overview/labels"
import { m } from "@/paraglide/messages.js"

import { known, type Concept, type ConceptProps } from "../concept"

/**
 * Observatory: no containers. Thin type straight on deep space, numbers as
 * the headline, a starfield and film grain for depth. The node column fades
 * the globe out behind it instead of boxing it.
 */
function Hud({ networkName, nodeCount, summary, selected, onSelect }: ConceptProps) {
  const counts = known(summary)
  return (
    <div className="lab-observatory pointer-events-none fixed inset-0">
      <div className="absolute top-8 left-10">
        <div className="lab-caps">Meshway</div>
        <div className="mt-2 text-xl font-light tracking-tight">{networkName}</div>
      </div>

      <div className="absolute bottom-10 left-10">
        <div className="flex items-end gap-14">
          <div>
            <div className="text-[76px] leading-[0.85] font-extralight tracking-tighter tabular-nums">{nodeCount}</div>
            <div className="lab-caps mt-3">{m.stat_nodes({ count: nodeCount })}</div>
          </div>
          <div>
            <div className="text-[76px] leading-[0.85] font-extralight tracking-tighter tabular-nums">
              {counts.confirmed}
              <span className="text-[40px] text-white/40">/{counts.total}</span>
            </div>
            <div className="lab-caps mt-3">{m.stat_links({ count: counts.total })}</div>
          </div>
        </div>
        <div className="lab-caps mt-6 flex gap-6">
          {[
            [lookColor.one_sided, m.stat_one_sided({ count: String(counts.oneSided) }), counts.oneSided],
            [lookColor.degraded, m.stat_degraded({ count: String(counts.degraded) }), counts.degraded],
            [lookColor.configured, m.stat_unreported({ count: String(counts.unreported) }), counts.unreported],
          ].map(([color, text, value]) => (
            <span key={String(text)} className={Number(value) > 0 ? "flex items-center gap-2" : "flex items-center gap-2 opacity-40"}>
              <span className="size-1 rounded-full" style={{ background: String(color) }} />
              {text}
            </span>
          ))}
        </div>
      </div>

      {selected ? (
        <aside key={selected.id} className="lab-scrim node-card-enter pointer-events-auto absolute inset-y-0 right-0 w-[440px] pt-28 pr-10 pl-24">
          <button
            type="button"
            aria-label={m.close()}
            onClick={() => onSelect(null)}
            className="absolute top-8 right-8 rounded-full p-2 text-white/60 transition hover:text-white"
          >
            <XIcon className="size-4" />
          </button>
          <div className="lab-caps">{m.lab_node()}</div>
          <h2 className="mt-3 text-[34px] leading-none font-light tracking-tight">{selected.name}</h2>
          <div className="mt-4 flex items-center gap-2 text-sm text-white/70">
            <span className="size-1.5 rounded-full" style={{ background: vitalityColor[selected.vitality], boxShadow: `0 0 12px ${vitalityColor[selected.vitality]}` }} />
            {selected.vitalityText}
          </div>

          <dl className="mt-10 grid grid-cols-2 gap-x-8 gap-y-6">
            {[
              [m.node_mesh_address(), selected.ipv4, true],
              [m.node_version(), selected.version, true],
              [m.node_report(), selected.report, false],
              [m.node_contact(), selected.contact, false],
              [m.node_receipt(), selected.receipt.text, true],
              [m.node_location(), selected.location, false],
            ].map(([label, value, mono]) => (
              <div key={String(label)}>
                <dt className="lab-caps">{label}</dt>
                <dd className={mono ? "mt-2 font-mono text-[13px]" : "mt-2 text-[13px]"}>{value}</dd>
              </div>
            ))}
          </dl>

          <div className="lab-caps mt-12">{m.node_links()}</div>
          <ul className="mt-3">
            {selected.links.map((link) => (
              <li key={link.key} className="border-t border-white/10 first:border-t-0">
                <button type="button" onClick={() => onSelect(link.peerId)} className="flex w-full items-center gap-4 py-3 text-left transition hover:text-white">
                  <span className="h-px w-5" style={{ background: lookColor[link.look] }} />
                  <span className="flex-1 text-sm">{link.peer}</span>
                  <span className="font-mono text-xs text-white/50">{link.own ?? link.lookText}</span>
                </button>
              </li>
            ))}
          </ul>
        </aside>
      ) : null}
    </div>
  )
}

export const observatory: Concept = {
  id: "observatory",
  name: () => m.lab_observatory(),
  theme: "dark",
  panelWidth: 440,
  look: { land: "dots", landSize: 1.25, graticule: false, stars: true, guides: false, rim: 0.6, atmosphere: 0.1, surfaceOpacity: 1 },
  Hud,
}
