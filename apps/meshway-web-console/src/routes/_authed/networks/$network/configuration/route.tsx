import type { ConfigurationValidationParams } from "@meshway/control-client"
import { createFileRoute } from "@tanstack/react-router"
import { useCallback } from "react"

import { useNetworkSummary } from "@/api/queries/network"
import { useConfigurationCommit } from "@/components/configuration/commit"
import { PageFrame } from "@/components/surface/page-frame"
import { isSection, sectionOf } from "@/lib/configuration/changes"
import type { DraftInput } from "@/lib/configuration/draft-state"
import { formViewOf } from "@/lib/configuration/forms"

import {
  useConfigurationNavigation,
  useConfigurationRoute,
} from "./-navigation"
import { ConfigurationPanel } from "./-panel"
import { HistoryWorkspace } from "./$view/-history/history-workspace"
import { SectionWorkspace } from "./$view/-sections/section-workspace"
import { SourceWorkspace } from "./$view/-source/source-workspace"

/** The configuration's sections, and one view over them (its child route). */
export const Route = createFileRoute(
  "/_authed/networks/$network/configuration"
)({
  staticData: { page: "configuration" },
  component: ConfigurationPage,
})

/**
 * The configuration page: the sections for every member who may read them,
 * the JSONC source and the revisions for those who edit. One draft per
 * sign-in and network, kept across pages; the commit bar applies it, always
 * through the controller's check and a review.
 */
function ConfigurationPage() {
  const { network: networkId } = Route.useParams()
  const network = useNetworkSummary(networkId)
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
