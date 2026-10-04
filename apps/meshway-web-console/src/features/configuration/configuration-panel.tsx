import type { ConfigurationView, NetworkSummary } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import {
  AtSignIcon,
  BinaryIcon,
  FileCodeIcon,
  HistoryIcon,
  ListFilterIcon,
  RouteIcon,
  ServerIcon,
  ShieldCheckIcon,
  SlidersHorizontalIcon,
  TagIcon,
  UsersIcon,
  WaypointsIcon,
  type LucideIcon,
} from "lucide-react"

import { DraftMark } from "@/components/chrome/draft-mark"
import { ListNote, ListRow, PanelList } from "@/components/chrome/list"
import { PanelHeader } from "@/components/chrome/panel-header"
import { Skeleton } from "@/components/ui/skeleton"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { m } from "@/paraglide/messages.js"
import { useConfigurationDraft } from "@/state/configuration-draft"

import { entryCount, SECTIONS, type SectionKey } from "./changes"
import { holdsDocument, operationsOf } from "./draft-state"
import { FORM_PARTS_OF, isFormSection } from "./forms"
import { sectionLabel } from "./labels"
import { edits } from "./operations"
import { configurationQuery, isDenied } from "./queries"
import type { ConfigurationViewId } from "./url"

const sectionIcon: Record<SectionKey, LucideIcon> = {
  access: ShieldCheckIcon,
  topology: WaypointsIcon,
  routing: RouteIcon,
  groups: UsersIcon,
  tagsOwners: TagIcon,
  dns: AtSignIcon,
  addressPools: BinaryIcon,
  nodes: ServerIcon,
  ruleSets: ListFilterIcon,
  defaults: SlidersHorizontalIcon,
}

export function SectionGlyph({ section }: { section: SectionKey }) {
  const Icon = sectionIcon[section]
  return <Icon className="size-4 text-muted-foreground" />
}

/** What a section holds now, in its own terms; no summary beyond the values. */
function sectionFact(view: ConfigurationView, section: SectionKey): string {
  switch (section) {
    case "dns":
      return view.dns.domain
    case "addressPools":
      return [...view.addressPools.ipv4, ...view.addressPools.ipv6].join(", ")
    case "routing":
      return m.configuration_routing_counts({
        mesh: view.routing.mesh?.length ?? 0,
        egress: view.routing.egress?.rules?.length ?? 0,
      })
    case "defaults":
      return m.configuration_defaults_values({
        access: view.defaults.access,
        topology: view.defaults.topology,
      })
    default:
      return m.configuration_entries({
        count: entryCount(view[section]) ?? 0,
      })
  }
}

/** The configuration page's index: the source and history for those who edit, then the sections. */
export function ConfigurationPanel({
  networkId,
  network,
  openView,
  onOpen,
}: {
  networkId: string
  /** null until read. */
  network: NetworkSummary | null
  openView: ConfigurationViewId | null
  onOpen: (view: ConfigurationViewId) => void
}) {
  const configuration = useQuery(configurationQuery(networkId))
  const draft = useConfigurationDraft(networkId)
  const editable = network?.actions.editConfiguration ?? false
  // Source edits mark the source; form edits, the sections they change.
  const document = holdsDocument(draft.entry)
  const ops = operationsOf(draft.entry)
  const revision =
    configuration.data?.revision ?? network?.currentRevision ?? null
  const view = configuration.data?.configuration

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PanelHeader
        page="configuration"
        title={m.configuration_title()}
        count={revision === null ? null : m.revision({ revision })}
      />
      <PanelList aria-labelledby={surfaceTitleId("configuration", "panel")}>
        {editable ? (
          <>
            <ListRow
              rowId="source"
              current={openView === "source"}
              onClick={() => onOpen("source")}
              lead={<FileCodeIcon className="size-4 text-muted-foreground" />}
              title={
                <>
                  <span className="truncate">{m.configuration_source()}</span>
                  {document ? <DraftMark /> : null}
                </>
              }
              subtitle={
                document && draft.entry ? (
                  <span className="truncate text-draft">
                    {m.revision_draft({ revision: draft.entry.baseRevision })}
                  </span>
                ) : revision !== null ? (
                  <span className="truncate">{m.revision({ revision })}</span>
                ) : (
                  <Skeleton className="h-2.5 w-16 rounded-full" />
                )
              }
            />
            <ListRow
              rowId="history"
              current={openView === "history"}
              onClick={() => onOpen("history")}
              lead={<HistoryIcon className="size-4 text-muted-foreground" />}
              title={
                <span className="truncate">{m.configuration_history()}</span>
              }
              subtitle={
                revision !== null ? (
                  <span className="truncate">
                    {m.configuration_current_revision({ revision })}
                  </span>
                ) : (
                  <Skeleton className="h-2.5 w-16 rounded-full" />
                )
              }
            />
            <li aria-hidden="true" className="mx-3 my-1.5 h-px bg-divider" />
          </>
        ) : null}
        {isDenied(configuration.error) ? (
          <ListNote>{m.configuration_read_denied()}</ListNote>
        ) : configuration.isError && !view ? (
          <ListNote>{m.configuration_read_failed()}</ListNote>
        ) : (
          SECTIONS.map((section) => (
            <ListRow
              key={section}
              rowId={section}
              current={openView === section}
              onClick={() => onOpen(section)}
              lead={<SectionGlyph section={section} />}
              title={
                <>
                  <span className="truncate">{sectionLabel[section]()}</span>
                  {isFormSection(section) &&
                  FORM_PARTS_OF[section].some((part) => edits(ops, part)) ? (
                    <DraftMark />
                  ) : null}
                </>
              }
              subtitle={
                view ? (
                  <span className="truncate">{sectionFact(view, section)}</span>
                ) : (
                  <Skeleton className="h-2.5 w-24 rounded-full" />
                )
              }
            />
          ))
        )}
      </PanelList>
    </div>
  )
}
