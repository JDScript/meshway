import {
  deleteNode,
  revokeNodeKey,
  type BoundNodeKey,
  type ConfigurationAppliedResponse,
  type NodeInventoryResponse,
  type NodeInventoryView,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { BanIcon, Trash2Icon, Undo2Icon } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { authed, client, codeOf } from "@/api/client"
import { toastFailureWith } from "@/api/failure"
import {
  configurationQuery,
  configurationReads,
} from "@/api/queries/configuration"
import { networkQuery } from "@/api/queries/network"
import { nodesQuery } from "@/api/queries/nodes"
import { CopyButton } from "@/components/chrome/copy-button"
import { ConfirmDialog } from "@/components/chrome/glass-dialog"
import { DangerRow, Section } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import {
  holdsDocument,
  holdsOperations,
  isDirty,
  operationsOf,
} from "@/lib/configuration/draft-state"
import { isDeleted, withDeleted } from "@/lib/configuration/node-operations"
import { dateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"
import { attempts } from "@/state/attempts"
import { draftOf, useConfigurationDraft } from "@/state/configuration-draft"

import { algorithmLabel, purposeLabel, type KeyPurpose } from "../-labels"

/** A public key's hex, shortened for a tile; the copy button holds it whole. */
const shortKey = (hex: string) => `${hex.slice(0, 12)}…${hex.slice(-12)}`

/**
 * The node's two keys (each revocable for good where the controller offers
 * it: `canRevokeControlKey`, `canRevokeDataplaneKey`) and its permanent
 * deletion (`canDelete`); the controller checks again.
 */
export function NodeManage({
  networkId,
  node,
  onDeleted,
}: {
  networkId: string
  node: NodeInventoryView
  onDeleted: () => void
}) {
  return (
    <div className="flex flex-col gap-8">
      <NodeKey
        networkId={networkId}
        node={node}
        purpose="control"
        bound={node.keys.control}
        createdAt={node.controlKeyCreatedAt}
        revokedAt={node.controlKeyRevokedAt}
        canRevoke={node.canRevokeControlKey}
      />
      <NodeKey
        networkId={networkId}
        node={node}
        purpose="dataplane"
        bound={node.keys.dataplane}
        createdAt={node.dataplaneKeyCreatedAt}
        revokedAt={node.dataplaneKeyRevokedAt}
        canRevoke={node.canRevokeDataplaneKey}
      />
      {node.canDelete ? (
        <Deletion networkId={networkId} node={node} onDeleted={onDeleted} />
      ) : null}
    </div>
  )
}

function NodeKey({
  networkId,
  node,
  purpose,
  bound,
  createdAt,
  revokedAt,
  canRevoke,
}: {
  networkId: string
  node: NodeInventoryView
  purpose: KeyPurpose
  bound: BoundNodeKey
  createdAt: string
  revokedAt: string | null
  canRevoke: boolean
}) {
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  const revoke = useMutation({
    mutationFn: () =>
      authed((token) =>
        revokeNodeKey(
          client,
          { networkId, nodeId: node.id },
          { keyId: bound.id, expectedGeneration: bound.generation },
          { token }
        )
      ),
    onSuccess: () => {
      setConfirming(false)
      toast.success(
        m.nodes_key_revoke_done({
          key: purposeLabel[purpose](),
          name: node.hostname,
        })
      )
    },
    onError: (error) => {
      toastFailureWith({
        version_conflict: () => m.nodes_key_changed(),
        state_conflict: () => m.nodes_key_revoke_moot(),
        permission_denied: () => m.nodes_manage_denied(),
      })(error)
      // A newer key generation: the dialog stays, over the key read again.
      const code = codeOf(error)
      if (code !== "version_conflict") setConfirming(false)
    },
    onSettled: () =>
      queryClient.invalidateQueries({
        queryKey: nodesQuery(networkId).queryKey,
      }),
  })
  const label = purposeLabel[purpose]()
  const hex = bound.publicKey.bytes

  return (
    <Section
      title={label}
      action={
        canRevoke ? (
          <Button
            variant="ghost"
            size="sm"
            className="-mr-3 h-8 rounded-full px-3 text-destructive hover:text-destructive"
            onClick={() => setConfirming(true)}
          >
            <BanIcon />
            {m.nodes_key_revoke()}
          </Button>
        ) : null
      }
    >
      <div className="grid grid-cols-2 gap-2 xl:grid-cols-4">
        <Tile label={m.nodes_key_algorithm()}>
          {algorithmLabel[bound.publicKey.algorithm]}
        </Tile>
        <Tile label={m.nodes_key_generation()}>
          <span className="tabular-nums">{bound.generation}</span>
        </Tile>
        <div className="col-span-2">
          <Tile label={m.nodes_key_created_at()}>
            {dateTime(createdAt) ?? "—"}
          </Tile>
        </div>
        <div className="col-span-2">
          <Tile
            label={m.nodes_key_state()}
            tone={revokedAt === null ? undefined : "warning"}
          >
            {revokedAt === null
              ? m.nodes_key_valid()
              : m.nodes_key_revoked_at({ at: dateTime(revokedAt) ?? "—" })}
          </Tile>
        </div>
        <div className="col-span-2 xl:col-span-4">
          <Tile
            label={m.nodes_key_public()}
            action={<CopyButton value={hex} label={m.nodes_key_public()} />}
          >
            <span className="font-mono font-normal">{shortKey(hex)}</span>
          </Tile>
        </div>
      </div>
      <ConfirmDialog
        open={confirming}
        onOpenChange={(open) => {
          if (!revoke.isPending) setConfirming(open)
        }}
        title={m.nodes_key_revoke_title({ key: label })}
        confirmLabel={m.nodes_key_revoke()}
        pending={revoke.isPending}
        onConfirm={() => revoke.mutate()}
        effects={[
          m.nodes_key_revoke_effect({
            key: label,
            generation: String(bound.generation),
            name: node.hostname,
          }),
          m.nodes_key_revoke_effect_execution(),
          m.nodes_key_revoke_effect_final(),
        ]}
      />
    </Section>
  )
}

type DeleteBody = { expectedRevision: number }

/** By network and node. */
const deletions = attempts<DeleteBody>("node-delete")

/** The draft holds edits not applied yet: deleting now would move its base under it. */
const draftHeld = (networkId: string) => isDirty(draftOf(networkId))

function Deletion({
  networkId,
  node,
  onDeleted,
}: {
  networkId: string
  node: NodeInventoryView
  onDeleted: () => void
}) {
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  // The revision the deletion is sent against: read again when it is asked.
  const configuration = useQuery({
    ...configurationQuery(networkId),
    enabled: confirming,
  })

  const refresh = () =>
    Promise.all(
      [
        nodesQuery(networkId).queryKey,
        configurationReads(networkId),
        networkQuery(networkId).queryKey,
      ].map((queryKey) => queryClient.invalidateQueries({ queryKey }))
    )

  const remove = useMutation({
    mutationFn: (revision: number): Promise<ConfigurationAppliedResponse> =>
      deletions.send(
        `${networkId}/${node.id}`,
        `${revision}`,
        () => ({ expectedRevision: revision }),
        (attempt) =>
          authed((token) =>
            deleteNode(
              client,
              { networkId, nodeId: node.id },
              { requestId: attempt.requestId, ...attempt.body },
              { token }
            )
          )
      ),
    retry: false,
    onSuccess: (applied) => {
      setConfirming(false)
      if (!applied.deletedNodeIds.includes(node.id)) {
        toast.error(m.nodes_delete_not_reported({ name: node.hostname }))
        void refresh()
        return
      }
      queryClient.setQueryData<NodeInventoryResponse>(
        nodesQuery(networkId).queryKey,
        (list) =>
          list && { items: list.items.filter((item) => item.id !== node.id) }
      )
      void refresh()
      toast.success(m.nodes_delete_done({ name: node.hostname }))
      onDeleted()
    },
    onError: (error) => {
      toastFailureWith({
        revision_conflict: () => m.nodes_delete_moved(),
        permission_denied: () => m.nodes_manage_denied(),
      })(error)
      const code = codeOf(error)
      if (code === "revision_conflict") void configuration.refetch()
      else if (code !== null) setConfirming(false)
    },
  })

  const draft = useConfigurationDraft(networkId)
  const deleting = isDeleted(operationsOf(draft.entry), node.id)

  /**
   * With form edits in the draft, the deletion joins them (`deleteNode`,
   * applied and reviewed with the rest); source edits refuse it; with no
   * draft it is sent now.
   */
  const ask = () => {
    const entry = draftOf(networkId)
    if (holdsDocument(entry)) {
      toast.error(m.nodes_delete_draft())
      return
    }
    if (holdsOperations(entry)) {
      const current = queryClient.getQueryData(
        configurationQuery(networkId).queryKey
      )
      if (!current) return
      draft.editOps(
        { revision: current.revision, configuration: current.configuration },
        (ops) => withDeleted(ops, node.id, true)
      )
      toast.success(m.nodes_delete_joined({ name: node.hostname }))
      return
    }
    void queryClient.invalidateQueries({
      queryKey: configurationQuery(networkId).queryKey,
    })
    setConfirming(true)
  }

  const read = configuration.data
  const ready = read !== undefined && !configuration.isFetching

  return (
    <Section title={m.nodes_deletion()}>
      {deleting ? (
        <DangerRow
          fact={m.nodes_delete_in_draft()}
          action={
            <Button
              variant="secondary"
              className="h-9 rounded-full px-4"
              onClick={() => {
                const current = queryClient.getQueryData(
                  configurationQuery(networkId).queryKey
                )
                if (!current) return
                draft.editOps(
                  {
                    revision: current.revision,
                    configuration: current.configuration,
                  },
                  (ops) => withDeleted(ops, node.id, false)
                )
              }}
            >
              <Undo2Icon />
              {m.nodes_delete_undo()}
            </Button>
          }
        />
      ) : (
        <DangerRow
          fact={m.nodes_delete_fact()}
          action={
            <Button
              variant="destructive"
              className="h-9 rounded-full px-4"
              onClick={ask}
            >
              <Trash2Icon />
              {m.nodes_delete()}
            </Button>
          }
        />
      )}
      <ConfirmDialog
        open={confirming}
        onOpenChange={(open) => {
          if (!remove.isPending) setConfirming(open)
        }}
        title={m.nodes_delete_title({ name: node.hostname })}
        confirmLabel={m.nodes_delete()}
        pending={remove.isPending}
        ready={ready}
        onConfirm={() => {
          if (!read) return
          if (draftHeld(networkId)) {
            setConfirming(false)
            toast.error(m.nodes_delete_draft())
            return
          }
          remove.mutate(read.revision)
        }}
        effects={[
          m.nodes_delete_effect_permanent({ name: node.hostname }),
          m.nodes_delete_effect_keys(),
          m.nodes_delete_effect_addresses(),
          m.nodes_delete_effect_again(),
        ]}
      />
    </Section>
  )
}
