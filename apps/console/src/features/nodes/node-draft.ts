import type {
  ConfigurationOperation,
  ConfigurationView,
  NetworkSummary,
  NodeSettingsView,
} from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { useCallback, useMemo } from "react"

import { useConfigurationDraft } from "@/features/configuration/draft"
import {
  checkedNow,
  holdsDocument,
  operationsOf,
} from "@/features/configuration/draft-state"
import { notesFrom, type Note } from "@/features/configuration/form-kit"
import { within } from "@/features/configuration/form-problems"
import { pointerOf } from "@/features/configuration/jsonc"
import { nodeProblems } from "@/features/configuration/node-problems"
import {
  configurationQuery,
  isDenied,
  memberConfigurationQuery,
} from "@/features/configuration/queries"
import { m } from "@/paraglide/messages.js"

/**
 * A node's settings as its workspace reads and edits them. Owners and
 * admins read the whole configuration and edit through the one draft (over
 * its base while it holds operations); a member has only their own nodes'
 * settings, from the event stream, read only.
 */
export type NodeConfiguration =
  | { kind: "loading" }
  | { kind: "failed"; error: unknown }
  /** Read but without this node (deleted, or not yet in this revision). */
  | { kind: "missing" }
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
  | { kind: "member"; settings: NodeSettingsView | null }

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
  const draft = useConfigurationDraft(networkId)
  const entry = draft.entry
  const { editOps } = draft

  const state: NodeConfiguration = useMemo(() => {
    if (denied) {
      const nodes = member.data?.nodes
      return {
        kind: "member",
        settings:
          nodes && Object.hasOwn(nodes, nodeId)
            ? (nodes[nodeId] ?? null)
            : null,
      }
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
          message: m.configuration_requirement({
            kind:
              typeof requirement.kind === "string"
                ? requirement.kind
                : requirement.kind.material,
          }),
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
    member.data,
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
