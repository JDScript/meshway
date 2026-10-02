import { UserRoundIcon, XIcon } from "lucide-react"

import { BrandMark } from "@/components/chrome/brand"
import { CopyButton } from "@/components/chrome/copy-button"
import { Button } from "@/components/ui/button"
import { lookColor, vitalityColor } from "@/features/overview/labels"
import { m } from "@/paraglide/messages.js"

import { known, type Concept, type ConceptProps } from "../concept"

/**
 * Aurora: soft colour fields drifting behind frosted glass. Everything sits
 * in one floating dock at the bottom; the node opens as a sheet beside it.
 */
function Hud({ networkName, nodeCount, summary, selected, onSelect }: ConceptProps) {
  const counts = known(summary)
  const segments = [
    { key: "confirmed", value: counts.confirmed, color: lookColor.confirmed },
    { key: "one_sided", value: counts.oneSided, color: lookColor.one_sided },
    { key: "degraded", value: counts.degraded, color: lookColor.degraded },
    { key: "configured", value: counts.unreported, color: lookColor.configured },
  ]
  return (
    <div className="pointer-events-none fixed inset-0">
      <div className="absolute inset-x-0 bottom-8 flex justify-center">
        <div className="glass pointer-events-auto flex h-16 items-center gap-6 rounded-full pr-3 pl-5">
          <span className="flex items-center gap-2.5">
            <BrandMark className="size-5" />
            <span className="text-sm font-medium">{networkName}</span>
          </span>
          <span className="h-6 w-px bg-foreground/10" />
          <span className="flex items-baseline gap-1.5">
            <span className="text-xl font-semibold tabular-nums">{nodeCount}</span>
            <span className="text-xs text-muted-foreground">{m.stat_nodes({ count: nodeCount })}</span>
          </span>
          <span className="flex items-center gap-3">
            <span className="flex items-baseline gap-1.5">
              <span className="text-xl font-semibold tabular-nums">
                {counts.confirmed}
                <span className="text-muted-foreground">/{counts.total}</span>
              </span>
              <span className="text-xs text-muted-foreground">{m.stat_links({ count: counts.total })}</span>
            </span>
            <span className="flex h-1.5 w-24 gap-0.5 overflow-hidden rounded-full bg-foreground/8">
              {segments.map((segment) =>
                segment.value > 0 ? <span key={segment.key} className="h-full rounded-full" style={{ flexGrow: segment.value, background: segment.color }} /> : null
              )}
            </span>
          </span>
          <span className="flex size-10 items-center justify-center rounded-full bg-foreground/8">
            <UserRoundIcon className="size-4" />
          </span>
        </div>
      </div>

      {selected ? (
        <aside key={selected.id} className="glass node-card-enter pointer-events-auto absolute top-8 right-8 bottom-32 w-[380px] overflow-y-auto rounded-[32px] p-7">
          <div className="flex items-start justify-between">
            <div>
              <h2 className="text-2xl font-semibold tracking-tight">{selected.name}</h2>
              <p className="mt-2 flex items-center gap-2 text-xs text-muted-foreground">
                <span className="size-2 rounded-full" style={{ background: vitalityColor[selected.vitality], boxShadow: `0 0 10px ${vitalityColor[selected.vitality]}` }} />
                {selected.vitalityText}
              </p>
            </div>
            <Button variant="ghost" size="icon-sm" className="rounded-full" aria-label={m.close()} onClick={() => onSelect(null)}>
              <XIcon />
            </Button>
          </div>

          <div className="mt-6 grid grid-cols-2 gap-2">
            {[
              [m.node_report(), selected.report],
              [m.node_contact(), selected.contact],
              [m.node_receipt(), selected.receipt.text],
              [m.node_version(), selected.version],
            ].map(([label, value]) => (
              <div key={label} className="rounded-2xl bg-foreground/5 px-3.5 py-3">
                <div className="text-[11px] text-muted-foreground">{label}</div>
                <div className="mt-1 truncate text-[13px] font-medium">{value}</div>
              </div>
            ))}
          </div>

          <div className="mt-2 rounded-2xl bg-foreground/5 px-3.5 py-3">
            <div className="text-[11px] text-muted-foreground">{m.node_mesh_address()}</div>
            {[selected.ipv4, selected.ipv6].map((address) => (
              <div key={address} className="mt-1 flex items-center justify-between font-mono text-[13px]">
                {address}
                <CopyButton value={address} label={address} />
              </div>
            ))}
          </div>

          <div className="mt-6 text-xs text-muted-foreground">{m.node_links()}</div>
          <ul className="mt-2 flex flex-col gap-1">
            {selected.links.map((link) => (
              <li key={link.key}>
                <button type="button" onClick={() => onSelect(link.peerId)} className="flex w-full items-center gap-3 rounded-2xl px-3 py-2.5 text-left transition hover:bg-foreground/5">
                  <span className="size-2 rounded-full" style={{ background: lookColor[link.look] }} />
                  <span className="flex-1 text-[13px]">{link.peer}</span>
                  <span className="font-mono text-xs text-muted-foreground">{link.own ?? link.lookText}</span>
                </button>
              </li>
            ))}
          </ul>
        </aside>
      ) : null}
    </div>
  )
}

export const aurora: Concept = {
  id: "aurora",
  name: () => m.lab_aurora(),
  theme: null,
  panelWidth: 420,
  look: { land: "dots", landSize: 1.45, graticule: false, stars: false, guides: false, rim: 0.9, atmosphere: 0.16, surfaceOpacity: 1 },
  Hud,
}
