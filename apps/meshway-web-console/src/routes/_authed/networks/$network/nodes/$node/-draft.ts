import type {
  ConfigurationOperation,
  ConfigurationView,
  NetworkSummary,
  NodeSettingsView,
} from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { useCallback, useMemo } from "react"

import { codeOf } from "@/api/client"
import {
  configurationQuery,
  isDenied,
  memberConfigurationQuery,
} from "@/api/queries/configuration"
import { nodeSettingsQuery } from "@/api/queries/nodes"
import { notesFrom, type Note } from "@/components/configuration/form-kit"
import {
  checkedNow,
  holdsDocument,
  operationsOf,
} from "@/lib/configuration/draft-state"
import { within } from "@/lib/configuration/form-problems"
import { pointerOf } from "@/lib/configuration/jsonc"
import { nodeProblems } from "@/lib/configuration/node-problems"
import { requirementText } from "@/lib/configuration/preparation"
import { useConfigurationDraft } from "@/state/configuration-draft"

/**
 * A node's settings as its workspace reads and edits them. Owners and
 * admins read the whole configuration and edit through the one draft (over
 * its base while it holds operations); a member has only their own nodes'
 * settings, read only: from the event stream once it has sent them, else
 * the controller's read of the node's settings.
 */
export type NodeConfiguration =
  | { kind: "loading" }
  | { kind: "failed"; error: unknown }
  /** Read but without this node (deleted, or not yet in this revision). */
  | { kind: "missing" }
  /** A member's read of a node that is not one of their own User-identity devices. */
  | { kind: "unreadable" }
  | {
      kind: "manager"
      /** What the draft's values are over. */
      base: ConfigurationView
      /** The current configuration: the base of a draft started here. */
      view: { revision: number; configuration: ConfigurationView }
      ops: readonly ConfigurationOperation[]
      /** The source holds edits: forms read only until they go. */
      document: boolean
      editable: boolean
      notes: Note[]
    }
  | { kind: "member"; settings: NodeSettingsView }

export function useNodeConfiguration(
  networkId: string,
  nodeId: string,
  network: NetworkSummary | null
) {
  const configuration = useQuery(configurationQuery(networkId))
  const denied = isDenied(configuration.error)
  const member = useQuery({
    ...memberConfigurationQuery(networkId),
    enabled: denied,
  })
  const streamed = member.data?.nodes
  const fromStream =
    streamed && Object.hasOwn(streamed, nodeId)
      ? (streamed[nodeId] ?? null)
      : null
  const read = useQuery({
    ...nodeSettingsQuery(networkId, nodeId),
    enabled: denied && fromStream === null,
  })
  const draft = useConfigurationDraft(networkId)
  const entry = draft.entry
  const { editOps } = draft

  const state: NodeConfiguration = useMemo(() => {
    if (denied) {
      if (fromStream) return { kind: "member", settings: fromStream }
      if (read.data) return { kind: "member", settings: read.data.settings }
      if (read.isError)
        return codeOf(read.error) === "not_found"
          ? { kind: "unreadable" }
          : { kind: "failed", error: read.error }
      return { kind: "loading" }
    }
    const view = configuration.data
    if (!view)
      return configuration.isError
        ? { kind: "failed", error: configuration.error }
        : { kind: "loading" }
    const base =
      entry?.input.kind === "operations" ? entry.input.base : view.configuration
    if (!Object.hasOwn(base.nodes, nodeId)) return { kind: "missing" }
    const ops = operationsOf(entry)
    const document = holdsDocument(entry)
    const prefix = pointerOf(["nodes", nodeId])
    const current = checkedNow(entry)
    const checked =
      current ??
      (entry?.checked?.input.kind === "operations" ? entry.checked : null)
    const notes = notesFrom(
      nodeProblems(ops, base).filter((problem) => within(problem.path, prefix)),
      [
        ...(checked?.issues ?? []),
        ...(checked?.requirements ?? []).map((requirement) => ({
          path: requirement.path,
          message: requirementText(requirement),
        })),
      ].filter((issue) => within(issue.path, prefix)),
      current === null
    )
    return {
      kind: "manager",
      base,
      view: { revision: view.revision, configuration: view.configuration },
      ops,
      document,
      editable: (network?.actions.editConfiguration ?? false) && !document,
      notes,
    }
  }, [
    denied,
    fromStream,
    read.data,
    read.isError,
    read.error,
    nodeId,
    configuration.data,
    configuration.isError,
    configuration.error,
    entry,
    network,
  ])

  /** Changes the draft's operations; refused (no change) while the source holds edits. */
  const change = useCallback(
    (
      edit: (
        ops: readonly ConfigurationOperation[],
        base: ConfigurationView
      ) => ConfigurationOperation[]
    ) => {
      if (state.kind !== "manager" || !state.editable) return
      editOps(state.view, edit)
    },
    [state, editOps]
  )

  return { state, change, discard: draft.discard }
}
