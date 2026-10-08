import { flexsearchFromSource } from "fumadocs-core/search/flexsearch"

import { source } from "@/lib/source"

// The search index, built once at build time: Chinese takes the CJK
// tokenizer.
export const revalidate = false

export const { staticGET: GET } = flexsearchFromSource(source, {
  localeMap: { zh: "cjk" },
})
