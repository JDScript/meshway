import { sectionOf, type SectionKey } from "./changes.ts"
import { pointerSegments } from "./jsonc.ts"
import type { FormPart } from "./operations.ts"

// Which sections have a form, which parts of the configuration each edits,
// and which form a controller diagnostic belongs to.

export type FormSection = Extract<
  SectionKey,
  "access" | "topology" | "groups" | "tagsOwners" | "dns" | "addressPools"
>

export const FORM_PARTS_OF: Record<FormSection, readonly FormPart[]> = {
  access: ["access"],
  // The topology's default (`defaults.topology`) is edited with its rules.
  topology: ["topology", "topologyDefault"],
  groups: ["groups"],
  tagsOwners: ["tagsOwners"],
  dns: ["dns"],
  addressPools: ["addressPools"],
}

export const isFormSection = (section: string): section is FormSection =>
  Object.hasOwn(FORM_PARTS_OF, section)

/** The form a JSON Pointer is shown in; null when no form shows it. */
export function formViewOf(pointer: string): FormSection | null {
  const segments = pointerSegments(pointer) ?? []
  if (segments[0] === "defaults")
    return segments[1] === "topology" ? "topology" : null
  // A node's addresses outside the pools are the pools' to show.
  if (segments[0] === "nodes" && segments[2] === "addresses")
    return "addressPools"
  const section = sectionOf(pointer)
  return isFormSection(section) ? section : null
}
