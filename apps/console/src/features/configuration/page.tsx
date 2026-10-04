import type {
  ConfigurationValidationParams,
  NetworkSummary,
} from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { useCallback, useMemo } from "react"

import { useCommit } from "@/features/shell/commit-bar"
import { PageFrame } from "@/features/shell/page-frame"
import { m } from "@/paraglide/messages.js"

import { isSection } from "./changes"
import { ConfigurationPanel } from "./configuration-panel"
import { useConfigurationDraft } from "./draft"
import { isOutdated, type DraftInput } from "./draft-state"
import { operationsProblems } from "./form-problems"
import { formViewOf } from "./forms"
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
  // Refused: the source at its problems, or the form holding the first one.
  const onInvalid = useCallback(
    (input: DraftInput, params: ConfigurationValidationParams) => {
      if (input.kind === "document") {
        if (view !== "source") openView("source")
        return
      }
      const first = params.issues[0]?.path ?? params.requirements[0]?.path
      const section = first === undefined ? null : formViewOf(first)
      if (section !== null && section !== view) openView(section)
    },
    [view, openView]
  )
  const { openRevision } = nav
  const onShowProgress = useCallback(
    (revision: number) => openRevision({ revision, against: null }),
    [openRevision]
  )
  const apply = useConfigurationApply(networkId, { onInvalid, onShowProgress })

  const entry = draft.entry
  const outdated = isOutdated(entry, current)
  const input = draft.dirty ? (entry?.input ?? null) : null
  // A document that does not parse, or form fields that will not save, stop applying.
  const blocked = useMemo(() => {
    if (input === null) return null
    if (input.kind === "document")
      return parseJsonc(input.text).ok ? null : m.configuration_commit_invalid()
    const count = operationsProblems(input.ops, input.base).length
    return count === 0 ? null : m.configuration_commit_fields({ count })
  }, [input])
  /** Checks the draft, against the controller's revision once it moved on (a new review). */
  const checkDraft = () => {
    if (!entry || blocked !== null) return
    void apply.check({
      type: "draft",
      input: entry.input,
      expectedRevision:
        outdated && current !== undefined ? current : entry.baseRevision,
    })
  }
  useCommit(
    editable && draft.dirty && entry
      ? {
          page: "configuration",
          label: m.configuration_commit_label(),
          tone: outdated ? "warning" : blocked ? "destructive" : "plain",
          text:
            outdated && current !== undefined
              ? m.configuration_commit_outdated({ current })
              : blocked
                ? blocked
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
          primaryDisabled: blocked !== null,
          busy: apply.checking,
          // Starting over reads the current configuration; discarding keeps the base.
          secondary: outdated ? draft.reset : draft.discard,
          primary: checkDraft,
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
        checking={apply.checking}
        onApplyDraft={checkDraft}
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
        onEditSource={(at) => nav.openView("source", at)}
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
