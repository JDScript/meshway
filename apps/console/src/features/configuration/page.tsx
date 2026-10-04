import type { NetworkSummary } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { useCallback, useMemo } from "react"

import { useCommit } from "@/features/shell/commit-bar"
import { PageFrame } from "@/features/shell/page-frame"
import { m } from "@/paraglide/messages.js"

import { isSection } from "./changes"
import { ConfigurationPanel } from "./configuration-panel"
import { useConfigurationDraft } from "./draft"
import { isOutdated } from "./draft-state"
import { HistoryWorkspace } from "./history-workspace"
import { parseJsonc } from "./jsonc"
import { useConfigurationNavigation, useConfigurationRoute } from "./navigation"
import { configurationQuery } from "./queries"
import { RefusalDialog, ReviewDialog } from "./review-dialog"
import { SectionWorkspace } from "./section-workspace"
import { SourceWorkspace } from "./source-workspace"
import { useConfigurationApply } from "./use-apply"

/**
 * The configuration page: the sections for every member who may read them,
 * the JSONC source and the revisions for those who edit. One draft per
 * sign-in and network, kept across pages; the commit bar applies it, always
 * through the controller's check and a review.
 */
export function ConfigurationPage({
  networkId,
  network,
}: {
  networkId: string
  /** null until read. */
  network: NetworkSummary | null
}) {
  const route = useConfigurationRoute()
  const nav = useConfigurationNavigation(networkId)
  const draft = useConfigurationDraft(networkId)
  // Read while the page is open; other pages leave it be.
  const configuration = useQuery({
    ...configurationQuery(networkId),
    enabled: route.current,
  })
  const current = configuration.data?.revision ?? network?.currentRevision
  const editable = network?.actions.editConfiguration ?? false

  const view = route.view
  const { openView } = nav
  const onInvalidDocument = useCallback(() => {
    if (view !== "source") openView("source")
  }, [view, openView])
  const { openRevision } = nav
  const onShowProgress = useCallback(
    (revision: number) => openRevision({ revision, against: null }),
    [openRevision]
  )
  const apply = useConfigurationApply(networkId, {
    onInvalidDocument,
    onShowProgress,
  })

  const entry = draft.entry
  const outdated = isOutdated(entry, current)
  const text = draft.dirty ? (entry?.input.text ?? null) : null
  const invalid = useMemo(() => text !== null && !parseJsonc(text).ok, [text])
  useCommit(
    editable && draft.dirty && entry
      ? {
          page: "configuration",
          label: m.configuration_commit_label(),
          tone: outdated ? "warning" : invalid ? "destructive" : "plain",
          text:
            outdated && current !== undefined
              ? m.configuration_commit_outdated({ current })
              : invalid
                ? m.configuration_commit_invalid()
                : apply.checking
                  ? m.configuration_checking()
                  : m.configuration_commit_unsaved(),
          secondaryLabel:
            outdated && current !== undefined
              ? m.configuration_commit_start_over({ current })
              : m.configuration_commit_discard(),
          primaryLabel:
            outdated && current !== undefined
              ? m.configuration_commit_recheck({ current })
              : m.configuration_commit_apply(),
          primaryDisabled: invalid,
          busy: apply.checking,
          // Starting over reads the current source; discarding keeps the base.
          secondary: outdated ? draft.reset : draft.discard,
          primary: () =>
            void apply.check({
              type: "document",
              sourceJsonc: entry.input.text,
              expectedRevision:
                outdated && current !== undefined
                  ? current
                  : entry.baseRevision,
            }),
        }
      : null
  )

  let workspace = null
  if (view === "source")
    workspace = (
      <SourceWorkspace
        networkId={networkId}
        network={network}
        at={route.at}
        onCompare={(from, to) =>
          nav.openRevision({ revision: to, against: from })
        }
        onClose={nav.closeView}
      />
    )
  else if (view === "history")
    workspace = (
      <HistoryWorkspace
        networkId={networkId}
        network={network}
        selection={route.history}
        checking={apply.checking}
        onSelect={nav.openRevision}
        onRestore={(revision, expectedRevision) =>
          void apply.check({
            type: "restore",
            restoreFrom: revision,
            expectedRevision,
          })
        }
        onClose={nav.closeView}
      />
    )
  else if (view !== null && isSection(view))
    workspace = (
      <SectionWorkspace
        key={view}
        networkId={networkId}
        network={network}
        section={view}
        onEdit={(at) => nav.openView("source", at)}
        onClose={nav.closeView}
      />
    )

  return (
    <>
      <PageFrame
        networkId={networkId}
        page="configuration"
        current={route.current}
        panel={
          <ConfigurationPanel
            active={route.current}
            networkId={networkId}
            network={network}
            openView={view}
            onOpen={(next) => nav.openView(next)}
          />
        }
        workspace={workspace}
        onCloseWorkspace={nav.closeView}
      />
      <ReviewDialog
        networkId={networkId}
        review={apply.review}
        applying={apply.applying}
        onConfirm={() => void apply.confirm()}
        onClose={apply.closeReview}
      />
      <RefusalDialog refusal={apply.refusal} onClose={apply.closeRefusal} />
    </>
  )
}
