import { sectionOf, type SectionKey } from "./changes"
import { pointerSegments } from "./jsonc"
import type { FormPart } from "./operations"

// Which sections have a form, which parts of the configuration each edits,
// and which form a controller diagnostic belongs to.

export type FormSection = Extract<
  SectionKey,
  | "access"
  | "topology"
  | "routing"
  | "groups"
  | "tagsOwners"
  | "dns"
  | "addressPools"
  | "ruleSets"
  | "defaults"
>

export const FORM_PARTS_OF: Record<FormSection, readonly FormPart[]> = {
  access: ["access"],
  // The topology's default (`defaults.topology`) is edited with its rules,
  // and with the dialer defaults: one `replaceDefaults` holds both.
  topology: ["topology", "defaults"],
  groups: ["groups"],
  tagsOwners: ["tagsOwners"],
  dns: ["dns"],
  addressPools: ["addressPools"],
  routing: ["routing"],
  ruleSets: ["ruleSets"],
  defaults: ["defaults"],
}

export const isFormSection = (section: string): section is FormSection =>
  Object.hasOwn(FORM_PARTS_OF, section)

/** The form a JSON Pointer is shown in; null when no form shows it. */
export function formViewOf(pointer: string): FormSection | null {
  const segments = pointerSegments(pointer) ?? []
  if (segments[0] === "defaults")
    return segments[1] === "topology" ? "topology" : "defaults"
  // A node's addresses outside the pools are the pools' to show.
  if (segments[0] === "nodes" && segments[2] === "addresses")
    return "addressPools"
  const section = sectionOf(pointer)
  return isFormSection(section) ? section : null
}
