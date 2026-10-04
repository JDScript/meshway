import { ChipSetField, type ChipKind } from "@/components/configuration/chips"
import {
  notesWithin,
  RemoveButton,
  RuleRow,
  setRefusal,
  type Note,
} from "@/components/configuration/form-kit"
import { pointerOf } from "@/lib/configuration/jsonc"
import { renamed, withEntry, without } from "@/lib/configuration/keyed"
import type { GroupMap } from "@/lib/configuration/operations"
import { m } from "@/paraglide/messages.js"

import { AddKey, KeyedRows, KeyInput, type KeyRule } from "./keyed-form"

/**
 * A map of named sets: `groups` (a group's members, by email) or
 * `tagsOwners` (who may give a tag: emails and groups). Rows in the map's
 * order; a name is renamed in place.
 */
export function GroupsForm({
  section,
  map,
  kind,
  editable,
  notes,
  onChange,
}: {
  section: "groups" | "tagsOwners"
  map: GroupMap
  kind: ChipKind
  editable: boolean
  notes: readonly Note[]
  onChange: (map: GroupMap) => void
}) {
  const groups = section === "groups"
  const rule: KeyRule = groups
    ? { prefix: "group:", placeholder: m.configuration_group_placeholder() }
    : { prefix: "tag:", placeholder: m.configuration_tag_placeholder() }
  const entries = Object.entries(map)

  return (
    <div className="flex flex-col gap-2">
      <KeyedRows
        count={entries.length}
        none={
          groups ? m.configuration_groups_none() : m.configuration_tags_none()
        }
        label={
          groups
            ? m.configuration_section_groups()
            : m.configuration_section_tagsOwners()
        }
      >
        {entries.map(([key, values], index) => {
          const at = pointerOf([section, key])
          return (
            // A row is its place: renaming keeps it.
            <li key={index}>
              <RuleRow
                label={key}
                notes={notesWithin(notes, at)}
                end={
                  editable ? (
                    <RemoveButton
                      label={m.configuration_key_remove({ name: key })}
                      onRemove={() => onChange(without(map, key))}
                    />
                  ) : null
                }
              >
                <KeyInput
                  map={map}
                  value={key}
                  rule={rule}
                  label={
                    groups
                      ? m.configuration_group_name()
                      : m.configuration_tag_name()
                  }
                  editable={editable}
                  onRename={(next) => onChange(renamed(map, key, next))}
                  className="w-44 shrink-0"
                />
                <ChipSetField
                  values={values}
                  kind={kind}
                  label={
                    groups
                      ? m.configuration_group_members()
                      : m.configuration_tag_owners()
                  }
                  addLabel={
                    groups
                      ? m.configuration_group_members_add()
                      : m.configuration_tag_owners_add()
                  }
                  editable={editable}
                  invalid={setRefusal(notes, at)}
                  onChange={(next) => onChange(withEntry(map, key, next))}
                />
              </RuleRow>
            </li>
          )
        })}
      </KeyedRows>
      {editable ? (
        <AddKey
          map={map}
          rule={rule}
          label={
            groups ? m.configuration_group_new() : m.configuration_tag_new()
          }
          addLabel={m.configuration_key_add()}
          onAdd={(key) => onChange(withEntry<string[]>(map, key, []))}
        />
      ) : null}
    </div>
  )
}
