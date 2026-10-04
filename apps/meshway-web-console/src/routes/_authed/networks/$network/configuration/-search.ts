import { z } from "zod"

import { SECTIONS, type SectionKey } from "@/lib/configuration/changes"

// The configuration page in the address; values at their default are left out.
//
//   /networks/<network>/configuration                     the sections
//   /networks/<network>/configuration/<section>           one section, read only
//   /networks/<network>/configuration/source?at=          the JSONC source (Owner, Admin)
//   /networks/<network>/configuration/history?revision=&against=
//                                                         the revisions, one of them, a comparison
//
// `at` is a JSON Pointer the source opens at.

const CONFIGURATION_VIEWS = [...SECTIONS, "source", "history"] as const
export type ConfigurationViewId = SectionKey | "source" | "history"

export const isConfigurationView = (
  value: string
): value is ConfigurationViewId =>
  CONFIGURATION_VIEWS.some((view) => view === value)

const zRevisionParam = z.coerce
  .number()
  .int()
  .min(1)
  .max(Number.MAX_SAFE_INTEGER)
  .optional()
  .catch(undefined)

export const zConfigurationSearch = z.object({
  at: z.string().max(4096).optional().catch(undefined),
  revision: zRevisionParam,
  against: zRevisionParam,
})
