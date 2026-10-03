import type { NetworkSummary } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { FileCodeIcon } from "lucide-react"

import { CloseButton } from "@/components/chrome/close-button"
import { RevisionLine } from "@/components/chrome/draft-mark"
import { IconAction } from "@/components/chrome/icon-action"
import { EmptyLine } from "@/components/chrome/section"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Skeleton } from "@/components/ui/skeleton"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { m } from "@/paraglide/messages.js"

import { JsonBlock } from "./change-list"
import { entryCount, type SectionKey } from "./changes"
import { pointerOf } from "./jsonc"
import { sectionLabel } from "./labels"
import { configurationQuery, isDenied } from "./queries"

/**
 * One section of the current configuration, read only, as the controller
 * returns it (non-secret). Those who edit open the source at this section.
 */
export function SectionWorkspace({
  networkId,
  network,
  section,
  onEdit,
  onClose,
}: {
  networkId: string
  /** null until read. */
  network: NetworkSummary | null
  section: SectionKey
  onEdit: (at: string) => void
  onClose: () => void
}) {
  const configuration = useQuery(configurationQuery(networkId))
  const value = configuration.data?.configuration[section]
  const count = value === undefined ? null : entryCount(value)
  const label = sectionLabel[section]()

  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col" key={section}>
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2
            id={surfaceTitleId("configuration", "workspace")}
            className={className}
          >
            {label}
          </h2>
        )}
        action={
          <>
            {network?.actions.editConfiguration ? (
              <IconAction
                label={m.configuration_edit_in_source()}
                icon={FileCodeIcon}
                onClick={() => onEdit(pointerOf([section]))}
              />
            ) : null}
            <CloseButton onClick={onClose} />
          </>
        }
        meta={
          <RevisionLine
            revision={
              configuration.data?.revision ?? network?.currentRevision ?? null
            }
            draftBase={null}
            facts={[
              count === null ? null : m.configuration_entries({ count }),
              m.read_only(),
            ].filter((fact) => fact !== null)}
          />
        }
      />
      <div className="scroll-edge min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-6">
        {isDenied(configuration.error) ? (
          <EmptyLine>{m.configuration_read_denied()}</EmptyLine>
        ) : configuration.isError && value === undefined ? (
          <EmptyLine>{m.configuration_read_failed()}</EmptyLine>
        ) : value === undefined ? (
          <Skeleton className="h-40 w-full rounded-tile" />
        ) : (
          <section aria-label={m.configuration_view_label({ section: label })}>
            <JsonBlock value={value} className="max-h-none text-row" />
          </section>
        )}
      </div>
    </div>
  )
}
