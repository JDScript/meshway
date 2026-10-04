import type { PreparedRuleSet } from "@meshway/control-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"

import { GlassDialog } from "@/components/chrome/glass-dialog"
import { EmptyLine, Section } from "@/components/chrome/section"
import { Tile } from "@/components/chrome/tile"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
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
import { revisionQuery } from "./queries"

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
export function ReviewDialog({
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
          <div className="grid gap-2 sm:grid-cols-3">
            <Tile label={m.configuration_review_count()}>
              <span className="tabular-nums">{preview.changeCount}</span>
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
export function RefusalDialog({
  refusal,
  onClose,
}: {
  refusal: Refusal | null
  onClose: () => void
}) {
  const params = refusal?.params
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
            message: issue.message,
          })),
          ...(params?.requirements ?? []).map((requirement) => ({
            path: requirement.path,
            message: m.configuration_requirement({
              kind:
                typeof requirement.kind === "string"
                  ? requirement.kind
                  : requirement.kind.material,
            }),
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
        refusal={apply.refusal}
        onClose={() => closeRefusal(networkId)}
      />
    </>
  )
}
