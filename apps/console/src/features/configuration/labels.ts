import type {
  ActivityActor,
  ChangeKind,
  ConfigurationChangeCause,
} from "@meshway/control-client"

import { m } from "@/paraglide/messages.js"

import { isSection, type SectionKey } from "./changes"

export const sectionLabel: Record<SectionKey, () => string> = {
  access: () => m.configuration_section_access(),
  topology: () => m.configuration_section_topology(),
  routing: () => m.configuration_section_routing(),
  groups: () => m.configuration_section_groups(),
  tagsOwners: () => m.configuration_section_tagsOwners(),
  dns: () => m.configuration_section_dns(),
  addressPools: () => m.configuration_section_addressPools(),
  nodes: () => m.configuration_section_nodes(),
  ruleSets: () => m.configuration_section_ruleSets(),
  defaults: () => m.configuration_section_defaults(),
}

/** A top-level key's name; one the console does not know is shown as written. */
export const sectionName = (section: string) =>
  section === ""
    ? m.configuration_section_document()
    : isSection(section)
      ? sectionLabel[section]()
      : section

export const causeLabel: Record<ConfigurationChangeCause, () => string> = {
  networkCreated: () => m.configuration_cause_networkCreated(),
  edit: () => m.configuration_cause_edit(),
  restore: () => m.configuration_cause_restore(),
  enrollment: () => m.configuration_cause_enrollment(),
  nodeDeleted: () => m.configuration_cause_nodeDeleted(),
  emailSync: () => m.configuration_cause_emailSync(),
  memberRemoved: () => m.configuration_cause_memberRemoved(),
  identityTransition: () => m.configuration_cause_identityTransition(),
}

export const changeKindLabel: Record<ChangeKind, () => string> = {
  added: () => m.configuration_change_added(),
  removed: () => m.configuration_change_removed(),
  changed: () => m.configuration_change_changed(),
}

/**
 * Who made a revision: this account, a member by email when the member
 * list is readable here, else the user's ID; or the controller itself.
 */
export function actorName(
  actor: ActivityActor,
  selfId: string | null,
  emails: ReadonlyMap<string, string>
): string {
  if (actor.type === "system") return m.configuration_actor_system()
  if (actor.userId === selfId) return m.configuration_actor_you()
  return (
    emails.get(actor.userId) ??
    m.configuration_actor_user({ id: actor.userId.slice(0, 8) })
  )
}
