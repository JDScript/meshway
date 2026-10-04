import type {
  ChangeKind,
  ConfigurationChangeCause,
} from "@meshway/control-client"

import {
  isSection,
  type BindingKey,
  type SectionKey,
} from "@/lib/configuration/changes"
import { m } from "@/paraglide/messages.js"

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
    : section === "bindings"
      ? m.configuration_section_bindings()
      : isSection(section)
        ? sectionLabel[section]()
        : section

/** What a change to a binding means; its value is the controller's own. */
export const bindingLabel: Record<BindingKey | "other", () => string> = {
  users: () => m.configuration_binding_users(),
  nodeUsers: () => m.configuration_binding_nodeUsers(),
  listeners: () => m.configuration_binding_listeners(),
  ruleSets: () => m.configuration_binding_ruleSets(),
  other: () => m.configuration_binding_other(),
}

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
