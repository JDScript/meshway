import {
  saveNetworkPolicy,
  type NetworkPolicy,
  type NetworkPolicySnapshot,
} from "@meshway/control-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useCallback, useMemo } from "react"

import { networkQuery, networksQuery } from "@/features/network/queries"
import { useCanManage } from "@/features/network/role"
import { useDraftCommit } from "@/features/shell/draft-commit"
import { createDraft, type Saved } from "@/features/shell/drafts"
import { authed } from "@/lib/api"
import { sameData } from "@/lib/same-data"

import {
  editPolicy,
  movePolicy,
  placeDiagnostic,
  policyText,
  readDocument,
  type PlacedDiagnostic,
  type PolicyDocument,
} from "./document"
import type { JsonPath } from "./jsonc"
import { problemMessage } from "./problems"
import { policyQuery } from "./queries"

/**
 * The policy draft is its JSONC text: the graphical views and the source
 * editor read and write the same text, against one revision. A policy
 * saved without its source text is written out from the policy itself.
 */
const usePolicyDraft = createDraft<string>("policy")

function savedPolicy(snapshot: NetworkPolicySnapshot): Saved<string> {
  return {
    revision: snapshot.revision,
    value: snapshot.source_jsonc ?? policyText(snapshot.policy),
  }
}

const POLICY_SECTIONS = [
  "connections",
  "grants",
  "mesh",
  "public_rules",
  "profiles",
  "automatic",
  "assignments",
] as const satisfies readonly (keyof NetworkPolicy)[]

/** The policy page's state, shared by its panel and workspaces. */
export function usePolicy(networkId: string, enabled = true) {
  const snapshot = useQuery({ ...policyQuery(networkId), enabled })
  const canManage = useCanManage(networkId)
  const data = snapshot.data
  const saved = useMemo(() => (data ? savedPolicy(data) : undefined), [data])
  const draft = usePolicyDraft(networkId, saved)
  const text = draft.value
  const document: PolicyDocument | null = useMemo(
    () => (text === undefined ? null : readDocument(text)),
    [text]
  )
  const savedValue = data?.policy ?? null

  const diagnostics: PlacedDiagnostic[] = useMemo(
    () =>
      (data?.diagnostics ?? []).map((diagnostic) =>
        placeDiagnostic(diagnostic, savedValue)
      ),
    [data, savedValue]
  )

  /** Sections the draft changes from what is saved. */
  const changed = useMemo(() => {
    const out = new Set<string>()
    if (!draft.dirty || !document?.policy) return out
    for (const section of POLICY_SECTIONS)
      if (!sameData(document.policy[section], savedValue?.[section]))
        out.add(section)
    return out
  }, [draft.dirty, document, savedValue])

  const { edit: editDraft } = draft
  const edit = useCallback(
    (update: (policy: NetworkPolicy) => NetworkPolicy) =>
      editDraft((current) => editPolicy(current, update)),
    [editDraft]
  )
  const move = useCallback(
    (path: JsonPath, index: number, delta: -1 | 1) =>
      editDraft((current) => movePolicy(current, path, index, delta)),
    [editDraft]
  )
  const setText = useCallback(
    (next: string) => editDraft(() => next),
    [editDraft]
  )

  return {
    snapshot,
    data,
    canManage,
    saved,
    draft,
    text: text ?? "",
    document,
    savedPolicy: savedValue,
    diagnostics,
    changed,
    editable: canManage,
    edit,
    move,
    setText,
  }
}

export type PolicyState = ReturnType<typeof usePolicy>

/** Offers the draft to the commit bar; only the open workspace calls this. */
export function usePolicyCommit(networkId: string, state: PolicyState) {
  const queryClient = useQueryClient()
  const first = state.document?.problems[0]
  return useDraftCommit({
    page: "policy",
    draft: state.draft,
    saved: state.saved,
    problem:
      first && state.document
        ? problemMessage(first, state.document.text)
        : null,
    save: (text, revision) => {
      const policy = readDocument(text).policy
      if (!policy) return Promise.reject(new Error("invalid policy text"))
      return authed((base, token) =>
        saveNetworkPolicy(base, token, networkId, {
          expected_revision: revision,
          policy,
          source_jsonc: text,
        })
      )
    },
    onSaved: (snapshot) => {
      queryClient.setQueryData(policyQuery(networkId).queryKey, snapshot)
      // The policy shares the network's revision.
      void queryClient.invalidateQueries({
        queryKey: networkQuery(networkId).queryKey,
      })
      void queryClient.invalidateQueries({ queryKey: networksQuery().queryKey })
    },
    refresh: () => void state.snapshot.refetch(),
  })
}
