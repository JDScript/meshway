import {
  claimNodeOwnership,
  revokeNode,
  setNodeTags,
  type NodeList,
  type NodeSummary,
  type TagSnapshot,
} from "@meshway/control-client"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { CheckIcon, ChevronDownIcon } from "lucide-react"
import { useMemo, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import { Tile } from "@/components/chrome/tile"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import {
  connectionsQuery,
  membersQuery,
  nodesQuery,
  ownershipQuery,
  runtimesQuery,
  tagsQuery,
} from "@/features/network/queries"
import { useNetworkRole } from "@/features/network/role"
import { useDraftCommit } from "@/features/shell/draft-commit"
import { authed } from "@/lib/api"
import { failureOf, toastFailure } from "@/lib/failure"
import { dateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { useNodeTagsDraft } from "./drafts"
import { EmptyLine, TabSection } from "./sections"
import { discoveriesQuery, managedListenersQuery } from "./queries"

const MAX_NODE_TAGS = 32

const assignedTags = (snapshot: TagSnapshot, nodeId: string) =>
  [
    ...(snapshot.assignments.find((entry) => entry.node_id === nodeId)?.tags ??
      []),
  ].sort()

/** A node's tags, owner and revocation. Members see the same, without controls. */
export function NodeManage({
  networkId,
  node,
  canManage,
}: {
  networkId: string
  node: NodeSummary
  canManage: boolean
}) {
  const active = node.revoked_at === null
  return (
    <div className="flex flex-col gap-8">
      <NodeTags
        key={node.id}
        networkId={networkId}
        node={node}
        canManage={canManage && active}
      />
      <Ownership networkId={networkId} node={node} active={active} />
      <Revocation networkId={networkId} node={node} canManage={canManage} />
    </div>
  )
}

function NodeTags({
  networkId,
  node,
  canManage,
}: {
  networkId: string
  node: NodeSummary
  canManage: boolean
}) {
  const queryClient = useQueryClient()
  const tags = useQuery(tagsQuery(networkId))
  const saved = useMemo(
    () =>
      tags.data
        ? {
            revision: tags.data.revision,
            value: assignedTags(tags.data, node.id),
          }
        : undefined,
    [tags.data, node.id]
  )
  const draft = useNodeTagsDraft(networkId, node.id, saved)
  const chosen = new Set(draft.value ?? [])
  const tooMany = chosen.size > MAX_NODE_TAGS

  const commit = useDraftCommit({
    page: "nodes",
    draft,
    saved,
    problem: tooMany
      ? m.node_tags_too_many({ max: String(MAX_NODE_TAGS) })
      : null,
    save: (value, revision) =>
      authed((base, token) =>
        setNodeTags(base, token, networkId, {
          expected_revision: revision,
          node_id: node.id,
          tags: value,
        })
      ),
    onSaved: (snapshot) => {
      queryClient.setQueryData(tagsQuery(networkId).queryKey, snapshot)
      void queryClient.invalidateQueries({
        queryKey: nodesQuery(networkId).queryKey,
      })
    },
    refresh: () => void tags.refetch(),
  })

  const toggle = (tag: string) =>
    draft.edit((current) =>
      current.includes(tag)
        ? current.filter((entry) => entry !== tag)
        : [...current, tag].sort()
    )

  return (
    <TabSection
      title={m.node_tags()}
      count={draft.value ? String(chosen.size) : null}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault()
          commit.submit()
        }}
      >
        <button type="submit" hidden />
        {!tags.data ? (
          tags.isError ? (
            <EmptyLine>{m.nodes_tags_failed()}</EmptyLine>
          ) : (
            <Skeleton className="h-12 w-full rounded-tile" />
          )
        ) : tags.data.tags.length === 0 ? (
          <EmptyLine>{m.nodes_tags_none()}</EmptyLine>
        ) : (
          <fieldset className="flex flex-wrap gap-1.5">
            <legend className="sr-only">{m.node_tags()}</legend>
            {tags.data.tags.map((definition) => {
              const on = chosen.has(definition.tag)
              return (
                <button
                  key={definition.tag}
                  type="button"
                  aria-pressed={on}
                  disabled={!canManage}
                  title={definition.description || undefined}
                  onClick={() => toggle(definition.tag)}
                  className="inline-flex h-8 items-center gap-1.5 rounded-full border border-border px-3 font-mono text-xs transition-colors outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none aria-pressed:border-transparent aria-pressed:bg-fill-selected aria-pressed:text-foreground aria-pressed:hover:bg-fill-selected [&:disabled:not([aria-pressed=true])]:opacity-60 [&:not([aria-pressed=true])]:text-muted-foreground"
                >
                  {on ? <CheckIcon className="-ml-0.5 size-3.5" /> : null}
                  {definition.tag}
                </button>
              )
            })}
          </fieldset>
        )}
      </form>
    </TabSection>
  )
}

function Ownership({
  networkId,
  node,
  active,
}: {
  networkId: string
  node: NodeSummary
  active: boolean
}) {
  const queryClient = useQueryClient()
  const ownership = useQuery(ownershipQuery(networkId))
  const members = useQuery(membersQuery(networkId))
  const owner = useNetworkRole(networkId) === "owner"
  const entry = ownership.data?.nodes.find((item) => item.node_id === node.id)
  const member = members.data?.members.find(
    (item) => item.membership.membership_id === entry?.membership_id
  )
  const [choice, setChoice] = useState("")
  const [confirming, setConfirming] = useState(false)
  const target = members.data?.members.find(
    (item) => item.membership.membership_id === choice
  )

  const claim = useMutation({
    mutationFn: (membershipId: string) =>
      authed((base, token) =>
        claimNodeOwnership(base, token, networkId, {
          membership_id: membershipId,
          node_id: node.id,
        })
      ),
    onSuccess: () => {
      setConfirming(false)
      toast.success(m.node_owner_assigned())
    },
    onError: (error) => {
      setConfirming(false)
      toastFailure(error)
    },
    onSettled: () =>
      void queryClient.invalidateQueries({
        queryKey: ownershipQuery(networkId).queryKey,
      }),
  })

  const value = !ownership.data
    ? ownership.isError
      ? m.node_unknown()
      : null
    : !entry
      ? m.node_unknown()
      : entry.membership_id === null
        ? m.node_owner_none()
        : (member?.user.email ?? entry.membership_id)

  const assignable =
    owner && active && entry !== undefined && entry.membership_id === null

  return (
    <TabSection title={m.node_owner()}>
      <div className="flex flex-col gap-2 sm:flex-row sm:items-stretch">
        <div className="min-w-0 flex-1">
          <Tile label={m.node_owner_user()}>
            {value ?? <Skeleton className="h-4 w-32 rounded-full" />}
          </Tile>
        </div>
        {assignable ? (
          <form
            className="flex items-center gap-2 rounded-tile bg-fill-subtle py-2 pr-2 pl-2"
            onSubmit={(event) => {
              event.preventDefault()
              if (target) setConfirming(true)
            }}
          >
            <label className="sr-only" htmlFor={`owner-${node.id}`}>
              {m.node_owner_choose()}
            </label>
            <span className="relative">
              <select
                id={`owner-${node.id}`}
                name="owner"
                value={choice}
                onChange={(event) => setChoice(event.target.value)}
                className="h-9 w-56 appearance-none truncate rounded-[calc(var(--radius-tile)-4px)] bg-field pr-8 pl-3 text-row outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <option value="">{m.node_owner_choose()}</option>
                {members.data?.members.map((item) => (
                  <option
                    key={item.membership.membership_id}
                    value={item.membership.membership_id}
                  >
                    {item.user.email}
                  </option>
                ))}
              </select>
              <ChevronDownIcon className="pointer-events-none absolute top-1/2 right-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
            </span>
            <Button
              type="submit"
              className="h-9 rounded-full px-4"
              disabled={!target}
            >
              {m.node_owner_assign()}
            </Button>
          </form>
        ) : null}
      </div>
      <ConfirmDialog
        open={confirming}
        onOpenChange={(open) => {
          if (!claim.isPending) setConfirming(open)
        }}
        title={m.node_owner_assign_title()}
        confirmLabel={m.node_owner_assign()}
        pending={claim.isPending}
        onConfirm={() =>
          target && claim.mutate(target.membership.membership_id)
        }
      >
        {m.node_owner_assign_effect({
          node: node.name,
          user: target?.user.email ?? "",
        })}
      </ConfirmDialog>
    </TabSection>
  )
}

function Revocation({
  networkId,
  node,
  canManage,
}: {
  networkId: string
  node: NodeSummary
  canManage: boolean
}) {
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  const revoke = useMutation({
    mutationFn: () =>
      authed((base, token) =>
        revokeNode(base, token, networkId, { node_id: node.id })
      ),
    onSuccess: ({ node: revoked }) => {
      setConfirming(false)
      queryClient.setQueryData<NodeList>(
        nodesQuery(networkId).queryKey,
        (list) =>
          list && {
            nodes: list.nodes.map((entry) =>
              entry.id === revoked.id ? revoked : entry
            ),
          }
      )
      for (const query of [
        nodesQuery(networkId),
        connectionsQuery(networkId),
        runtimesQuery(networkId),
        tagsQuery(networkId),
        ownershipQuery(networkId),
        managedListenersQuery(networkId),
        discoveriesQuery(networkId),
      ])
        void queryClient.invalidateQueries({ queryKey: query.queryKey })
      toast.success(m.node_revoked_toast({ name: revoked.name }))
    },
    onError: (error) => {
      setConfirming(false)
      toastFailure(error)
      if (failureOf(error) === "missing")
        void queryClient.invalidateQueries({
          queryKey: nodesQuery(networkId).queryKey,
        })
    },
  })

  if (node.revoked_at !== null)
    return (
      <TabSection title={m.node_revocation()}>
        <Tile label={m.node_revoked_at()} tone="warning">
          {dateTime(node.revoked_at) ?? node.revoked_at}
        </Tile>
      </TabSection>
    )
  if (!canManage) return null
  return (
    <TabSection title={m.node_revocation()}>
      <div className="flex min-h-12 items-center gap-3 rounded-tile bg-fill-subtle py-2 pr-2 pl-3.5">
        <span className="min-w-0 flex-1 text-row">{m.node_revoke_fact()}</span>
        <Button
          variant="destructive"
          className="h-9 rounded-full px-4"
          onClick={() => setConfirming(true)}
        >
          {m.node_revoke()}
        </Button>
      </div>
      <ConfirmDialog
        open={confirming}
        onOpenChange={(open) => {
          if (!revoke.isPending) setConfirming(open)
        }}
        title={m.node_revoke_title({ name: node.name })}
        confirmLabel={m.node_revoke()}
        pending={revoke.isPending}
        onConfirm={() => revoke.mutate()}
      >
        {m.node_revoke_effect()}
      </ConfirmDialog>
    </TabSection>
  )
}
