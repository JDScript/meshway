import type { AccessRule } from "@meshway/control-client"
import { ArrowLeftRightIcon, ArrowRightIcon } from "lucide-react"

import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { m } from "@/paraglide/messages.js"

import { ChipSetField } from "./chips"
import {
  AddButton,
  NoneLine,
  notesWithin,
  protocolChips,
  refusedAt,
  RemarkInput,
  RemoveButton,
  RuleRow,
  selectorChips,
  type Names,
  type Note,
} from "./form-kit"
import { pointerOf } from "./jsonc"

const replaceAt = <T,>(items: readonly T[], index: number, next: T): T[] =>
  items.map((item, at) => (at === index ? next : item))

/** Who may reach what: allows only, added up; order means nothing, so rules are not reordered. */
export function AccessForm({
  rules,
  editable,
  names,
  notes,
  onChange,
}: {
  rules: readonly AccessRule[]
  editable: boolean
  names: Names
  notes: readonly Note[]
  onChange: (rules: AccessRule[]) => void
}) {
  const sources = selectorChips(names, false)
  const destinations = selectorChips(names, true)
  const protocols = protocolChips()
  const update = (index: number, next: AccessRule) =>
    onChange(replaceAt(rules, index, next))

  return (
    <div className="flex flex-col gap-2">
      {rules.length === 0 ? (
        <NoneLine>{m.configuration_access_none()}</NoneLine>
      ) : null}
      <ul
        aria-label={m.configuration_section_access()}
        className="flex flex-col gap-2"
      >
        {rules.map((rule, index) => {
          const at = (field: string) => pointerOf(["access", index, field])
          return (
            // Rules have no IDs: a row is its position.
            <li key={index}>
              <RuleRow
                label={m.configuration_rule_label({ index: index + 1 })}
                notes={notesWithin(notes, pointerOf(["access", index]))}
                end={
                  <>
                    <RemarkInput
                      value={rule.remark ?? ""}
                      editable={editable}
                      onChange={(remark) => update(index, { ...rule, remark })}
                    />
                    {editable ? (
                      <RemoveButton
                        label={m.configuration_rule_remove({
                          index: index + 1,
                        })}
                        onRemove={() =>
                          onChange(rules.filter((_, at) => at !== index))
                        }
                      />
                    ) : null}
                  </>
                }
              >
                <ChipSetField
                  values={rule.src}
                  kind={sources}
                  label={m.configuration_access_src()}
                  addLabel={m.configuration_access_src_add()}
                  editable={editable}
                  invalid={refusedAt(notes, at("src"))}
                  onChange={(src) => update(index, { ...rule, src })}
                />
                {/* The direction keeps to the destinations when the row wraps. */}
                <span className="inline-flex min-w-0 flex-wrap items-center gap-1.5">
                  <Direction
                    both={rule.bidirectional ?? false}
                    editable={editable}
                    onChange={(bidirectional) =>
                      update(index, { ...rule, bidirectional })
                    }
                  />
                  <ChipSetField
                    values={rule.dst}
                    kind={destinations}
                    label={m.configuration_access_dst()}
                    addLabel={m.configuration_access_dst_add()}
                    editable={editable}
                    invalid={refusedAt(notes, at("dst"))}
                    onChange={(dst) => update(index, { ...rule, dst })}
                  />
                </span>
                <span
                  className="text-meta text-muted-foreground"
                  aria-hidden="true"
                >
                  {m.configuration_access_ip_join()}
                </span>
                <ChipSetField
                  values={rule.ip}
                  kind={protocols}
                  label={m.configuration_access_ip()}
                  addLabel={m.configuration_access_ip_add()}
                  editable={editable}
                  invalid={refusedAt(notes, at("ip"))}
                  onChange={(ip) => update(index, { ...rule, ip })}
                />
              </RuleRow>
            </li>
          )
        })}
      </ul>
      {editable ? (
        <AddButton
          label={m.configuration_access_add()}
          onAdd={() => onChange([...rules, { src: [], dst: [], ip: [] }])}
        />
      ) : null}
    </div>
  )
}

/** One way or both, between a rule's sources and destinations; pressing it turns it round. */
function Direction({
  both,
  editable,
  onChange,
}: {
  both: boolean
  editable: boolean
  onChange: (both: boolean) => void
}) {
  const Icon = both ? ArrowLeftRightIcon : ArrowRightIcon
  const label = both
    ? m.configuration_access_both()
    : m.configuration_access_one_way()
  if (!editable)
    return (
      <Icon
        role="img"
        aria-label={label}
        className="size-3.5 shrink-0 text-muted-foreground"
      />
    )
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type="button"
            aria-pressed={both}
            aria-label={m.configuration_access_bidirectional()}
            onClick={() => onChange(!both)}
            className="inline-flex size-7 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors outline-none hover:bg-fill-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring aria-pressed:text-foreground"
          />
        }
      >
        <Icon className="size-3.5" />
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}
