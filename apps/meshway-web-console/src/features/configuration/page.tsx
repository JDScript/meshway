import type {
  ConfigurationValidationParams,
  NetworkSummary,
} from "@meshway/control-client"
import { useCallback } from "react"

import { PageFrame } from "@/features/shell/page-frame"

import { isSection, sectionOf } from "./changes"
import { useConfigurationCommit } from "./commit"
import { ConfigurationPanel } from "./configuration-panel"
import type { DraftInput } from "./draft-state"
import { formViewOf } from "./forms"
import { HistoryWorkspace } from "./history-workspace"
import { useConfigurationNavigation, useConfigurationRoute } from "./navigation"
import { SectionWorkspace } from "./section-workspace"
import { SourceWorkspace } from "./source-workspace"

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
      const section =
        first === undefined
          ? null
          : (formViewOf(first) ??
            (sectionOf(first) === "nodes" ? "nodes" : null))
      if (section !== null && section !== view) openView(section)
    },
    [view, openView]
  )
  const commit = useConfigurationCommit({
    networkId,
    network,
    onInvalid,
  })
  const { checkDraft } = commit

  let workspace = null
  if (view === "source")
    workspace = (
      <SourceWorkspace
        networkId={networkId}
        network={network}
        at={route.at}
        checking={commit.checking}
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
        checking={commit.checking}
        onSelect={nav.openRevision}
        onRestore={(revision, expectedRevision) =>
          commit.check({
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
        onOpenSection={(section) => nav.openView(section)}
        onClose={nav.closeView}
      />
    )

  return (
    <>
      <PageFrame
        networkId={networkId}
        page="configuration"
        panel={
          <ConfigurationPanel
            networkId={networkId}
            network={network}
            openView={view}
            onOpen={(next) => nav.openView(next)}
          />
        }
        workspace={workspace}
        commit={commit.bar}
        onCloseWorkspace={nav.closeView}
      />
    </>
  )
}
