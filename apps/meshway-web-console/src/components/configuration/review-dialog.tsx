import type {
  ChangeKind,
  ConfigurationIssue,
  ListenerChange,
  PreparationRequirement,
  PreparedRuleSet,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { CircleAlertIcon } from "lucide-react"

import { configurationQuery, revisionQuery } from "@/api/queries/configuration"
import { GlassDialog } from "@/components/chrome/glass-dialog"
import { EmptyLine, Section } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import { Spinner } from "@/components/ui/spinner"
import {
  issueText,
  nodeNamer,
  type NodeNamer,
} from "@/lib/configuration/diagnostics"
import {
  listenerOf,
  preparationLabel,
  requirementText,
} from "@/lib/configuration/preparation"
import { m } from "@/paraglide/messages.js"
import {
  closeRefusal,
  closeReview,
  confirmConfiguration,
  useConfigurationApply,
  type Refusal,
  type Review,
} from "@/state/configuration-apply"

import { ChangeList } from "./change-list"

/** Nodes by hostname (as the base revision has them) and ID. */
function NodeIds({
  ids,
  names,
}: {
  ids: readonly string[]
  names: Readonly<Record<string, { hostname: string }>> | undefined
}) {
  if (ids.length === 0)
    return <EmptyLine size="sm">{m.configuration_review_none()}</EmptyLine>
  return (
    <ul className="flex flex-col gap-1">
      {ids.map((id) => {
        const hostname =
          names && Object.hasOwn(names, id) ? names[id]?.hostname : undefined
        return (
          <li
            key={id}
            className="flex min-w-0 items-baseline gap-2 rounded-tile bg-fill-subtle px-3 py-2"
          >
            {hostname !== undefined ? (
              <span className="shrink-0 text-row font-medium">{hostname}</span>
            ) : null}
            <span className="min-w-0 truncate font-mono text-meta text-muted-foreground">
              {id}
            </span>
          </li>
        )
      })}
    </ul>
  )
}

const CHANGE: Record<ChangeKind, () => string> = {
  added: () => m.configuration_review_listener_added(),
  removed: () => m.configuration_review_listener_removed(),
  changed: () => m.configuration_review_listener_changed(),
}

/**
 * The listeners applying adds, removes or changes, by node, with the
 * material the controller generates for them on apply and what it still has
 * to complete. The preview holds none of the material.
 */
function ListenerChanges({
  changes,
  willGenerate,
  requirements,
  nodes,
}: {
  changes: readonly ListenerChange[]
  willGenerate: readonly PreparationRequirement[]
  requirements: readonly PreparationRequirement[]
  nodes: Readonly<Record<string, { hostname: string }>> | undefined
}) {
  const listeners = new Map<
    string,
    {
      nodeId: string
      key: string
      kind: ChangeKind | null
      generate: string[]
      require: string[]
    }
  >()
  const entryOf = (nodeId: string, key: string) => {
    const id = `${nodeId}/${key}`
    const entry = listeners.get(id) ?? {
      nodeId,
      key,
      kind: null,
      generate: [],
      require: [],
    }
    listeners.set(id, entry)
    return entry
  }
  for (const change of changes)
    entryOf(change.nodeId, change.key).kind = change.kind
  const add = (
    items: readonly PreparationRequirement[],
    into: "generate" | "require"
  ) => {
    for (const item of items) {
      const at = listenerOf(item.path)
      if (!at) continue
      const entry = entryOf(at.nodeId, at.key)
      const label = preparationLabel(item.kind)
      if (!entry[into].includes(label)) entry[into].push(label)
    }
  }
  add(willGenerate, "generate")
  add(requirements, "require")
  return (
    <ul className="flex flex-col gap-1">
      {[...listeners.entries()].map(([id, entry]) => {
        const hostname =
          nodes && Object.hasOwn(nodes, entry.nodeId)
            ? nodes[entry.nodeId]?.hostname
            : undefined
        return (
          <li
            key={id}
            className="flex min-w-0 flex-col gap-0.5 rounded-tile bg-fill-subtle px-3 py-2"
          >
            <span className="flex min-w-0 items-baseline gap-2">
              <span className="text-row font-medium">
                {m.configuration_review_listener({ key: entry.key })}
              </span>
              <span className="min-w-0 truncate text-meta text-muted-foreground">
                {hostname ?? entry.nodeId}
              </span>
              {entry.kind ? (
                <span className="ml-auto shrink-0 text-meta text-muted-foreground">
                  {CHANGE[entry.kind]()}
                </span>
              ) : null}
            </span>
            {entry.generate.length > 0 ? (
              <span className="text-meta text-muted-foreground">
                {m.configuration_review_will_generate({
                  items: entry.generate.join(" · "),
                })}
              </span>
            ) : null}
            {entry.require.length > 0 ? (
              <span className="text-meta text-warning">
                {m.configuration_review_requires({
                  items: entry.require.join(" · "),
                })}
              </span>
            ) : null}
          </li>
        )
      })}
    </ul>
  )
}

/** What the controller accepted but warns may not be meant, by path. */
function Warnings({
  warnings,
  node,
}: {
  warnings: readonly ConfigurationIssue[]
  node: NodeNamer
}) {
  return (
    <ul className="flex flex-col gap-1">
      {warnings.map((warning, index) => (
        <li
          key={index}
          className="flex min-w-0 items-start gap-2 rounded-tile bg-fill-subtle px-3 py-2"
        >
          <CircleAlertIcon
            aria-hidden="true"
            className="mt-1 size-3 shrink-0 text-warning"
          />
          <span className="flex min-w-0 flex-col gap-0.5">
            <span className="text-row">{issueText(warning, node)}</span>
            <code className="font-mono text-meta break-all text-muted-foreground">
              {warning.path || m.configuration_problem_root()}
            </code>
          </span>
        </li>
      ))}
    </ul>
  )
}

/** How many nodes applying may change: all, none, or some of them. */
function affectedText(affected: number, total: number): string {
  if (affected === 0) return m.configuration_review_none()
  if (affected >= total)
    return m.configuration_review_affected_all({ count: total })
  return m.configuration_review_affected_some({ count: affected, total })
}

/** The remote rule sets the check downloaded and converted, by key: tentative, nothing saved. */
function PreparedRuleSets({
  sets,
}: {
  sets: Readonly<Record<string, PreparedRuleSet>>
}) {
  return (
    <ul className="flex flex-col gap-1">
      {Object.entries(sets).map(([key, set]) => (
        <li
          key={key}
          className="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-0.5 rounded-tile bg-fill-subtle px-3 py-2"
        >
          <span className="min-w-0 text-row font-medium break-all">{key}</span>
          <span className="text-meta text-muted-foreground tabular-nums">
            {m.configuration_review_rule_set_entries({ entries: set.entries })}
          </span>
          <span
            title={set.digest}
            className="font-mono text-meta text-muted-foreground"
          >
            {m.configuration_review_rule_set_digest({
              digest: set.digest.slice(0, 12),
            })}
          </span>
        </li>
      ))}
    </ul>
  )
}

/**
 * What applying would do, as the controller computed it: the effective
 * change count, permanent node deletions and address changes on their own,
 * the remote rule sets it prepared, then every change with its value before
 * and after (non-secret views).
 * Nothing is applied until it is confirmed here.
 */
function ReviewDialog({
  networkId,
  review,
  applying,
  onConfirm,
  onClose,
}: {
  networkId: string
  review: Review | null
  applying: boolean
  onConfirm: () => void
  onClose: () => void
}) {
  const preview = review?.preview
  const base = useQuery({
    ...revisionQuery(networkId, preview?.baseRevision ?? 1),
    enabled: preview !== undefined,
  })
  const restore =
    review?.proposal.type === "restore" ? review.proposal.restoreFrom : null
  const listeners = preview
    ? new Set([
        ...preview.changedListeners.map(
          (change) => `${change.nodeId}/${change.key}`
        ),
        ...[
          ...(preview.willGenerate ?? []),
          ...(preview.requirements ?? []),
        ].flatMap((item) => {
          const at = listenerOf(item.path)
          return at ? [`${at.nodeId}/${at.key}`] : []
        }),
      ]).size
    : 0
  const material =
    preview !== undefined &&
    (preview.willGenerate ?? []).length + (preview.requirements ?? []).length >
      0
  const node = nodeNamer({
    ...(base.data?.configuration.nodes ?? {}),
    ...(preview?.preview.nodes ?? {}),
  })
  const nothing =
    preview !== undefined && preview.changeCount === 0 && !preview.sourceChanged
  const note = !preview
    ? null
    : review?.unknown
      ? m.configuration_review_unknown()
      : nothing
        ? m.configuration_review_nothing({ revision: preview.baseRevision })
        : preview.changeCount === 0
          ? m.configuration_review_comments_only()
          : null

  return (
    <GlassDialog
      wide
      open={review !== null}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      busy={applying}
      title={
        restore !== null
          ? m.configuration_review_restore_title({ revision: restore })
          : m.configuration_review_title()
      }
      footer={
        <>
          <Button
            variant="ghost"
            className="h-9 min-w-20 rounded-full px-4"
            disabled={applying}
            onClick={onClose}
          >
            {m.dialog_cancel()}
          </Button>
          <Button
            className="h-9 min-w-24 rounded-full px-4"
            disabled={applying || nothing}
            aria-busy={applying}
            onClick={onConfirm}
          >
            {applying ? <Spinner /> : null}
            {restore !== null
              ? m.configuration_review_restore()
              : m.configuration_review_apply()}
          </Button>
        </>
      }
    >
      {preview ? (
        <>
          <div className="grid gap-2 sm:grid-cols-4">
            <Tile label={m.configuration_review_count()}>
              <span className="tabular-nums">{preview.changeCount}</span>
            </Tile>
            <Tile label={m.configuration_review_affected()}>
              <span className="tabular-nums">
                {affectedText(
                  preview.affectedNodeIds.length,
                  Object.keys(preview.preview.nodes).length
                )}
              </span>
            </Tile>
            <Tile label={m.configuration_review_base()}>
              <span className="font-mono">
                {m.revision({ revision: preview.baseRevision })}
              </span>
            </Tile>
            <Tile label={m.configuration_review_source()}>
              {preview.sourceChanged
                ? m.configuration_review_source_changed()
                : m.configuration_review_source_same()}
            </Tile>
          </div>
          {note ? (
            <p className="text-row text-muted-foreground">{note}</p>
          ) : null}
          {preview.warnings.length > 0 ? (
            <Section
              title={m.configuration_review_warnings()}
              count={preview.warnings.length}
            >
              <Warnings warnings={preview.warnings} node={node} />
            </Section>
          ) : null}
          <Section
            title={m.configuration_review_deleted()}
            count={preview.deletedNodeIds.length}
          >
            {preview.deletedNodeIds.length > 0 ? (
              <p className="text-row text-destructive">
                {m.configuration_review_deleted_permanent()}
              </p>
            ) : null}
            <NodeIds
              ids={preview.deletedNodeIds}
              names={base.data?.configuration.nodes}
            />
          </Section>
          <Section
            title={m.configuration_review_address()}
            count={preview.addressChangedNodeIds.length}
          >
            <NodeIds
              ids={preview.addressChangedNodeIds}
              names={base.data?.configuration.nodes}
            />
          </Section>
          <Section
            title={m.configuration_review_identity()}
            count={preview.identityChangedNodeIds.length}
          >
            <NodeIds
              ids={preview.identityChangedNodeIds}
              names={base.data?.configuration.nodes}
            />
          </Section>
          {Object.keys(preview.preparedRuleSets).length > 0 ? (
            <Section
              title={m.configuration_review_rule_sets()}
              count={Object.keys(preview.preparedRuleSets).length}
            >
              <p className="text-row text-muted-foreground">
                {m.configuration_review_rule_sets_tentative()}
              </p>
              <PreparedRuleSets sets={preview.preparedRuleSets} />
            </Section>
          ) : null}
          {listeners > 0 ? (
            <Section
              title={m.configuration_review_listeners()}
              count={listeners}
            >
              {material ? (
                <p className="text-row text-muted-foreground">
                  {m.configuration_review_material_note()}
                </p>
              ) : null}
              <ListenerChanges
                changes={preview.changedListeners}
                willGenerate={preview.willGenerate ?? []}
                requirements={preview.requirements ?? []}
                nodes={{
                  ...(base.data?.configuration.nodes ?? {}),
                  ...preview.preview.nodes,
                }}
              />
            </Section>
          ) : null}
          {preview.changes.length > 0 ? (
            <ChangeList
              changes={preview.changes}
              before={base.data?.configuration}
              after={preview.preview}
            />
          ) : null}
        </>
      ) : null}
    </GlassDialog>
  )
}

/** A restore the controller refused: what it said, by path. */
function RefusalDialog({
  networkId,
  refusal,
  onClose,
}: {
  networkId: string
  refusal: Refusal | null
  onClose: () => void
}) {
  const params = refusal?.params
  // Nodes by hostname, as the current revision has them.
  const current = useQuery({
    ...configurationQuery(networkId),
    enabled: refusal !== null,
  })
  const node = nodeNamer(current.data?.configuration.nodes ?? {})
  const restore =
    refusal?.proposal.type === "restore" ? refusal.proposal.restoreFrom : null
  return (
    <GlassDialog
      open={refusal !== null}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      title={
        restore !== null
          ? m.configuration_review_restore_title({ revision: restore })
          : m.configuration_review_refused()
      }
    >
      <p className="text-row text-destructive">
        {m.configuration_review_refused()}
      </p>
      <ul className="flex flex-col gap-1">
        {[
          ...(params?.issues ?? []).map((issue) => ({
            path: issue.path,
            message: issueText(issue, node),
          })),
          ...(params?.requirements ?? []).map((requirement) => ({
            path: requirement.path,
            message: requirementText(requirement),
          })),
        ].map((item, index) => (
          <li
            key={index}
            className="flex flex-col gap-0.5 rounded-tile bg-fill-subtle px-3 py-2"
          >
            <code className="font-mono text-meta text-muted-foreground">
              {item.path || m.configuration_problem_root()}
            </code>
            <span className="text-row">{item.message}</span>
          </li>
        ))}
      </ul>
    </GlassDialog>
  )
}

/**
 * The network's review and refusal, wherever the check was asked from:
 * mounted once by the network shell, so they outlive the page that asked.
 */
export function ConfigurationDialogs({ networkId }: { networkId: string }) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const apply = useConfigurationApply(networkId)
  const showProgress = (revision: number) =>
    void navigate({
      to: "/networks/$network/configuration/$view",
      params: { network: networkId, view: "history" },
      search: { revision },
    })
  return (
    <>
      <ReviewDialog
        networkId={networkId}
        review={apply.review}
        applying={apply.busy === "applying"}
        onConfirm={() =>
          void confirmConfiguration(queryClient, networkId, showProgress)
        }
        onClose={() => closeReview(networkId)}
      />
      <RefusalDialog
        networkId={networkId}
        refusal={apply.refusal}
        onClose={() => closeRefusal(networkId)}
      />
    </>
  )
}
