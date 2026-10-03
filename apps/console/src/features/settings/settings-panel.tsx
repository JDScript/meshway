import type { NetworkSummary } from "@meshway/control-client"
import { HistoryIcon, NetworkIcon, type LucideIcon } from "lucide-react"

import { ListRow, PanelList } from "@/components/chrome/list"
import { PanelHeader } from "@/components/chrome/panel-header"
import { Skeleton } from "@/components/ui/skeleton"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { m } from "@/paraglide/messages.js"

import { sectionOffered, SETTINGS_SECTIONS, type SettingsSection } from "./url"

export const sectionLabel: Record<SettingsSection, () => string> = {
  network: () => m.settings_section_network(),
  activity: () => m.settings_section_activity(),
}

const sectionIcon: Record<SettingsSection, LucideIcon> = {
  network: NetworkIcon,
  activity: HistoryIcon,
}

const summary: Record<SettingsSection, (network: NetworkSummary) => string> = {
  network: (network) => network.name,
  activity: () => m.settings_activity_summary(),
}

/** The settings page's index: one row per section, with what it holds now. */
export function SettingsPanel({
  network,
  openSection,
  onOpen,
}: {
  /** null until read. */
  network: NetworkSummary | null
  openSection: SettingsSection | null
  onOpen: (section: SettingsSection) => void
}) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <PanelHeader page="settings" title={m.nav_settings()} />
      <PanelList aria-labelledby={surfaceTitleId("settings", "panel")}>
        {SETTINGS_SECTIONS.filter(
          // Until the network is read, only what everyone may open.
          (section) =>
            sectionOffered(
              section,
              network?.actions ?? { manageMembers: false }
            )
        ).map((section) => {
          const Icon = sectionIcon[section]
          return (
            <ListRow
              key={section}
              rowId={section}
              current={section === openSection}
              onClick={() => onOpen(section)}
              lead={<Icon className="size-4 text-muted-foreground" />}
              title={
                <span className="truncate">{sectionLabel[section]()}</span>
              }
              subtitle={
                network ? (
                  <span className="truncate">{summary[section](network)}</span>
                ) : (
                  <Skeleton className="h-2.5 w-24 rounded-full" />
                )
              }
            />
          )
        })}
      </PanelList>
    </div>
  )
}
