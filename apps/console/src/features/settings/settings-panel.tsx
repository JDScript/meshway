import type { NetworkSnapshot } from "@meshway/control-client"
import { ChevronRightIcon } from "lucide-react"

import { Skeleton } from "@/components/ui/skeleton"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { m } from "@/paraglide/messages.js"

import { SETTINGS_SECTIONS, type SettingsSection } from "./url"

export const sectionLabel: Record<SettingsSection, () => string> = {
  network: () => m.settings_section_network(),
  dns: () => m.settings_section_dns(),
}

function summary(section: SettingsSection, network: NetworkSnapshot): string {
  if (section === "network") return network.name
  return network.dns.enabled
    ? `${network.dns.zone} · ${m.settings_dns_on()}`
    : m.settings_dns_off()
}

/** The settings page's index: one row per section, with what it holds now. */
export function SettingsPanel({
  network,
  openSection,
  onOpen,
}: {
  /** null until read (or when the read failed and nothing is cached). */
  network: NetworkSnapshot | null
  openSection: SettingsSection | null
  onOpen: (section: SettingsSection) => void
}) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="px-4 pt-4 pb-3">
        <h2
          id={surfaceTitleId("settings", "panel")}
          className="text-base font-semibold tracking-tight"
        >
          {m.nav_settings()}
        </h2>
      </header>
      <ul
        aria-labelledby={surfaceTitleId("settings", "panel")}
        className="min-h-0 flex-1 overflow-y-auto px-2 pb-2"
      >
        {SETTINGS_SECTIONS.map((section) => (
          <li key={section}>
            <button
              type="button"
              aria-current={section === openSection ? "true" : undefined}
              onClick={() => onOpen(section)}
              className="list-row h-14 gap-3 px-3"
            >
              <span className="min-w-0 flex-1">
                <span className="block truncate text-row font-medium">
                  {sectionLabel[section]()}
                </span>
                {network ? (
                  <span className="block truncate text-meta text-muted-foreground">
                    {summary(section, network)}
                  </span>
                ) : (
                  <Skeleton className="mt-1.5 h-2.5 w-24 rounded-full" />
                )}
              </span>
              <ChevronRightIcon className="size-4 shrink-0 text-muted-foreground" />
            </button>
          </li>
        ))}
      </ul>
    </div>
  )
}
