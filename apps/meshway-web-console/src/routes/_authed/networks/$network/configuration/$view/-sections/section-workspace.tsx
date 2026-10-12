import type { ConfigurationView, NetworkSummary } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { FileCodeIcon } from "lucide-react"
import { useMemo, type ReactNode } from "react"

import { configurationQuery, isDenied } from "@/api/queries/configuration"
import { ruleSetsQuery } from "@/api/queries/rule-sets"
import { CloseButton } from "@/components/chrome/close-button"
import { DraftMark, RevisionLine } from "@/components/chrome/draft-mark"
import { IconAction } from "@/components/chrome/icon-action"
import { EmptyLine } from "@/components/chrome/section"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { JsonBlock } from "@/components/configuration/change-list"
import {
  notesFrom,
  ownerChips,
  useNames,
  userChips,
  type Note,
} from "@/components/configuration/form-kit"
import { sectionLabel } from "@/components/configuration/labels"
import { DraftNotice } from "@/components/configuration/notice"
import { surfaceTitleId } from "@/components/surface/page-frame"
import { Skeleton } from "@/components/ui/skeleton"
import {
  entryCount,
  sectionOf,
  type SectionKey,
} from "@/lib/configuration/changes"
import { nodeNamer, saidOf } from "@/lib/configuration/diagnostics"
import {
  checkedNow,
  holdsDocument,
  holdsOperations,
  operationsOf,
} from "@/lib/configuration/draft-state"
import { partProblems } from "@/lib/configuration/form-problems"
import {
  FORM_PARTS_OF,
  formViewOf,
  isFormSection,
  type FormSection,
} from "@/lib/configuration/forms"
import { pointerOf, pointerSegments } from "@/lib/configuration/jsonc"
import { renamed } from "@/lib/configuration/keyed"
import { isDeleted } from "@/lib/configuration/node-operations"
import { nodeProblems } from "@/lib/configuration/node-problems"
import {
  edits,
  valueOf,
  type FormPart,
  type FormValues,
} from "@/lib/configuration/operations"
import {
  isProviderRef,
  referencedSets,
  renameSetReferences,
} from "@/lib/configuration/routing"
import { effectiveProviders } from "@/lib/configuration/rule-sets"
import { m } from "@/paraglide/messages.js"
import { useConfigurationDraft } from "@/state/configuration-draft"

import { AccessForm } from "./access-form"
import { AddressPoolsForm } from "./address-pools-form"
import { DefaultsForm } from "./defaults-form"
import { DnsForm } from "./dns-form"
import { GroupsForm } from "./groups-form"
import { NodesSection } from "./nodes-section"
import { RoutingForm } from "./routing-form"
import { RuleSetsForm, type RuleSetStatus } from "./rule-sets-form"
import { TopologyForm } from "./topology-form"

/**
 * One section of the configuration. Those with a form are edited in place
 * by those who edit the configuration (typed operations into the draft);
 * the rest read as the controller returns them (non-secret), and open the
 * source to be edited. Members who may not edit see the same form, read only.
 */
export function SectionWorkspace({
  networkId,
  network,
  section,
  onEditSource,
  onOpenSection,
  onClose,
}: {
  networkId: string
  /** null until read. */
  network: NetworkSummary | null
  section: SectionKey
  onEditSource: (at: string) => void
  onOpenSection: (section: SectionKey) => void
  onClose: () => void
}) {
  const configuration = useQuery(configurationQuery(networkId))
  const draft = useConfigurationDraft(networkId)
  const entry = draft.entry
  const editable = network?.actions.editConfiguration ?? false
  const label = sectionLabel[section]()
  const form = isFormSection(section) ? section : null
  const document = holdsDocument(entry)
  const ops = operationsOf(entry)
  // The forms show the draft over its own base; over source edits, the configuration as it is.
  const base =
    entry?.input.kind === "operations"
      ? entry.input.base
      : configuration.data?.configuration
  const edited =
    form !== null && FORM_PARTS_OF[form].some((part) => edits(ops, part))
  const value = base ? countOf(section, base, ops) : null

  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col" key={section}>
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2
            id={surfaceTitleId("configuration", "workspace")}
            className={className}
          >
            <span className="inline-flex items-center gap-2">
              {label}
              {edited ? <DraftMark /> : null}
            </span>
          </h2>
        )}
        action={
          <>
            {editable ? (
              <IconAction
                label={m.configuration_edit_in_source()}
                icon={FileCodeIcon}
                onClick={() => onEditSource(pointerOf([section]))}
              />
            ) : null}
            <CloseButton onClick={onClose} />
          </>
        }
        meta={
          <RevisionLine
            revision={
              configuration.data?.revision ?? network?.currentRevision ?? null
            }
            draftBase={
              holdsOperations(entry) && entry ? entry.baseRevision : null
            }
            facts={[
              value === null ? null : m.configuration_entries({ count: value }),
              editable && form !== null ? null : m.read_only(),
            ].filter((fact) => fact !== null)}
          />
        }
      />
      <div className="scroll-edge min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-[var(--commit-space,76px)]">
        {isDenied(configuration.error) ? (
          <EmptyLine>{m.configuration_read_denied()}</EmptyLine>
        ) : configuration.isError && !configuration.data ? (
          <EmptyLine>{m.configuration_read_failed()}</EmptyLine>
        ) : !base || !configuration.data ? (
          <Skeleton className="h-40 w-full rounded-tile" />
        ) : section === "nodes" ? (
          <NodesSection
            networkId={networkId}
            base={base}
            ops={ops}
            notes={nodeNotes(entry, base)}
          />
        ) : form === null ? (
          <section aria-label={m.configuration_view_label({ section: label })}>
            <JsonBlock value={base[section]} className="max-h-none text-row" />
          </section>
        ) : (
          <div className="flex flex-col gap-4">
            {editable && document ? (
              <DraftNotice
                text={m.configuration_notice_document()}
                primaryLabel={m.configuration_notice_open_source()}
                onPrimary={() => onEditSource(pointerOf([section]))}
                onDiscard={draft.discard}
              />
            ) : null}
            <SectionForm
              networkId={networkId}
              section={form}
              base={base}
              view={configuration.data}
              editable={editable && !document}
              readMembers={network?.actions.manageMembers ?? false}
              notes={notesFor(form, entry, base)}
              onChange={draft.editForm}
              onOpenSection={onOpenSection}
              onEditSource={onEditSource}
            />
          </div>
        )}
      </div>
    </div>
  )
}

/** How many entries the section holds in the draft. */
function countOf(
  section: SectionKey,
  base: ConfigurationView,
  ops: ReturnType<typeof operationsOf>
): number | null {
  if (section === "nodes")
    return Object.keys(base.nodes).filter((id) => !isDeleted(ops, id)).length
  if (!isFormSection(section)) return entryCount(base[section])
  if (section === "defaults") return null
  if (section === "addressPools") {
    const pools = valueOf(ops, base, "addressPools")
    return pools.ipv4.length + pools.ipv6.length
  }
  if (section === "dns") return null
  if (section === "routing") {
    const routing = valueOf(ops, base, "routing")
    return (
      (routing.mesh ?? []).length +
      (routing.egress?.rules ?? []).length +
      Object.values(routing.egress?.profiles ?? {}).reduce(
        (sum, profile) => sum + (profile.rules ?? []).length,
        0
      )
    )
  }
  return entryCount(valueOf(ops, base, section))
}

/** The problems found here in the draft's values, and what the controller said about them. */
function notesFor(
  form: FormSection,
  entry: ReturnType<typeof useConfigurationDraft>["entry"],
  base: ConfigurationView
): Note[] {
  const ops = operationsOf(entry)
  const local = FORM_PARTS_OF[form].flatMap((part) =>
    edits(ops, part) ? problemsOf(part, ops, base) : []
  )
  const current = checkedNow(entry)
  const checked =
    current ??
    (entry?.checked?.input.kind === "operations" ? entry.checked : null)
  const said = saidOf(checked, nodeNamer(base.nodes)).filter(
    (entry) =>
      formViewOf(entry.path) === form ||
      // Routing names rule sets: their content is checked with it.
      (form === "routing" && namesRuleSet(entry.path))
  )
  return notesFrom(local, said, current === null)
}

/** A rule set's content (`/ruleSets/<key>`). */
function namesRuleSet(path: string): boolean {
  const segments = pointerSegments(path) ?? []
  return segments[0] === "ruleSets" && segments.length === 2
}

/** The node problems found here, and what the controller said about the nodes. */
function nodeNotes(
  entry: ReturnType<typeof useConfigurationDraft>["entry"],
  base: ConfigurationView
): Note[] {
  const ops = operationsOf(entry)
  const current = checkedNow(entry)
  const checked =
    current ??
    (entry?.checked?.input.kind === "operations" ? entry.checked : null)
  return notesFrom(
    nodeProblems(ops, base),
    saidOf(checked, nodeNamer(base.nodes)).filter(
      (entry) => sectionOf(entry.path) === "nodes"
    ),
    current === null
  )
}

const problemsOf = <P extends FormPart>(
  part: P,
  ops: ReturnType<typeof operationsOf>,
  base: ConfigurationView
) => partProblems(part, valueOf(ops, base, part))

function SectionForm({
  networkId,
  section,
  base,
  view,
  editable,
  readMembers,
  notes,
  onChange,
  onOpenSection,
  onEditSource,
}: {
  networkId: string
  section: FormSection
  base: ConfigurationView
  /** The current configuration, the base of a draft started here. */
  view: { revision: number; configuration: ConfigurationView }
  editable: boolean
  readMembers: boolean
  notes: Note[]
  onChange: <P extends FormPart>(
    view: { revision: number; configuration: ConfigurationView },
    part: P,
    value: FormValues[P]
  ) => void
  onOpenSection: (section: SectionKey) => void
  /** Opens the source at a JSON pointer. */
  onEditSource: (at: string) => void
}) {
  const draft = useConfigurationDraft(networkId)
  const ops = operationsOf(draft.entry)
  const groups = valueOf(ops, base, "groups")
  const tags = valueOf(ops, base, "tagsOwners")
  const names = useNames(networkId, base, groups, tags, readMembers)
  const set =
    <P extends FormPart>(part: P) =>
    (value: FormValues[P]) =>
      onChange(view, part, value)
  const chips = useMemo(
    () => ({ users: userChips(names), owners: ownerChips(names) }),
    [names]
  )
  const statusRead = useQuery({
    ...ruleSetsQuery(networkId),
    enabled: section === "ruleSets",
  })
  const status: RuleSetStatus = statusRead.data
    ? {
        kind: "read",
        views: new Map(statusRead.data.items.map((item) => [item.key, item])),
      }
    : statusRead.isError
      ? { kind: "failed" }
      : { kind: "loading" }
  const ruleSets = valueOf(ops, base, "ruleSets")
  const routing = valueOf(ops, base, "routing")

  let body: ReactNode
  switch (section) {
    case "access":
      body = (
        <AccessForm
          rules={valueOf(ops, base, "access")}
          editable={editable}
          names={names}
          notes={notes}
          onChange={set("access")}
        />
      )
      break
    case "topology":
      body = (
        <TopologyForm
          rules={valueOf(ops, base, "topology")}
          fallback={valueOf(ops, base, "defaults").topology}
          editable={editable}
          names={names}
          notes={notes}
          onChange={set("topology")}
          onFallback={(topology) =>
            set("defaults")({ ...valueOf(ops, base, "defaults"), topology })
          }
        />
      )
      break
    case "groups":
      body = (
        <GroupsForm
          section="groups"
          map={groups}
          kind={chips.users}
          editable={editable}
          notes={notes}
          onChange={set("groups")}
        />
      )
      break
    case "tagsOwners":
      body = (
        <GroupsForm
          section="tagsOwners"
          map={tags}
          kind={chips.owners}
          editable={editable}
          notes={notes}
          onChange={set("tagsOwners")}
        />
      )
      break
    case "dns":
      body = (
        <DnsForm
          dns={valueOf(ops, base, "dns")}
          editable={editable}
          notes={notes}
          onChange={set("dns")}
        />
      )
      break
    case "routing": {
      body = (
        <RoutingForm
          routing={routing}
          editable={editable}
          names={names}
          notes={notes}
          exits={
            new Set(
              Object.entries(base.nodes)
                .filter(([, node]) => node.exitEnabled)
                .map(([id]) => id)
            )
          }
          ruleSets={[
            ...Object.keys(ruleSets),
            // Catalog sets already named, offered again (any other is typed).
            ...new Set(
              [
                ...referencedSets(base.routing),
                ...referencedSets(routing),
              ].filter(isProviderRef)
            ),
          ]}
          catalog={{
            networkId,
            providers: effectiveProviders(base.ruleSetProviders ?? {}),
          }}
          onChange={set("routing")}
          onOpenRuleSets={() => onOpenSection("ruleSets")}
        />
      )
      break
    }
    case "ruleSets":
      body = (
        <RuleSetsForm
          networkId={networkId}
          sets={ruleSets}
          saved={view.configuration.ruleSets}
          status={status}
          referenced={new Set(referencedSets(routing))}
          providers={base.ruleSetProviders ?? {}}
          editable={editable}
          notes={notes}
          onChange={set("ruleSets")}
          onEditProviders={() => onEditSource(pointerOf(["ruleSetProviders"]))}
          onRename={(from, to) => {
            onChange(view, "ruleSets", renamed(ruleSets, from, to))
            onChange(view, "routing", renameSetReferences(routing, from, to))
          }}
        />
      )
      break
    case "defaults":
      body = (
        <DefaultsForm
          value={valueOf(ops, base, "defaults")}
          access={base.defaults.access}
          editable={editable}
          notes={notes}
          onChange={set("defaults")}
        />
      )
      break
    case "addressPools":
      body = (
        <AddressPoolsForm
          pools={valueOf(ops, base, "addressPools")}
          editable={editable}
          notes={notes}
          onChange={set("addressPools")}
        />
      )
      break
  }
  return (
    <div className="flex flex-col" aria-label={sectionLabel[section]()}>
      {body}
    </div>
  )
}
