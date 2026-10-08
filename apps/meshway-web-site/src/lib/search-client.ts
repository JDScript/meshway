import { Charset, Document } from "flexsearch"
import {
  createContentHighlighter,
  type SortedResult,
} from "fumadocs-core/search"
import type { SearchClient } from "fumadocs-core/search/client"
import { z } from "zod"

import { i18n } from "./i18n"

// The static search, as Fumadocs' own static client but with each
// language's encoder: an index built with the CJK encoder (`api/search`)
// is only found by queries encoded the same way, which Fumadocs' client
// does not do.

type Entry = {
  id: string
  page_id: string
  type: "page" | "heading" | "text"
  content: string
  breadcrumbs?: string[]
  tags: string[]
  url: string
}

/** What `api/search` exports: each language's index, by key. */
const zExport = z.object({
  type: z.literal("i18n"),
  raw: z.record(z.string(), z.record(z.string(), z.string())),
})

const CJK = new Set<string>(["zh"])

function indexOf(locale: string, data: Record<string, string>) {
  const index = new Document<Entry, true>({
    tokenize: "full",
    ...(CJK.has(locale) ? { encoder: Charset.CJK } : {}),
    document: {
      id: "id",
      index: ["content"],
      tag: ["tags"],
      store: true,
    },
  })
  for (const [key, value] of Object.entries(data)) index.import(key, value)
  return index
}

let indexes: Promise<Map<string, Document<Entry, true>>> | null = null

async function load() {
  const response = await fetch("/api/search")
  if (!response.ok) throw new Error(`search index: ${response.status}`)
  const exported = zExport.parse(await response.json())
  return new Map(
    Object.entries(exported.raw).map(([locale, data]) => [
      locale,
      indexOf(locale, data),
    ])
  )
}

async function search(
  index: Document<Entry, true>,
  query: string
): Promise<SortedResult[]> {
  const found = await index.searchAsync(query, { index: "content", limit: 60 })
  const ids = found[0]?.result ?? []
  const highlighter = createContentHighlighter(query)
  const byPage = new Map<string, Entry[]>()
  for (const id of ids) {
    const entry = index.get(id)
    if (!entry) continue
    const list = byPage.get(entry.page_id) ?? []
    byPage.set(entry.page_id, list)
    if (entry.type !== "page") list.push(entry)
  }
  const out: SortedResult[] = []
  for (const [pageId, entries] of byPage) {
    const page = index.get(pageId)
    if (!page) continue
    out.push({
      id: pageId,
      type: "page",
      content: highlighter.highlightMarkdown(page.content),
      breadcrumbs: page.breadcrumbs,
      url: page.url,
    })
    for (const entry of entries)
      out.push({
        id: entry.id,
        type: entry.type,
        content: highlighter.highlightMarkdown(entry.content),
        breadcrumbs: entry.breadcrumbs,
        url: entry.url,
      })
  }
  return out
}

/** Searches the docs of `locale` (the default language when none). */
export function staticSearch(locale: string | undefined): SearchClient {
  const lang = locale ?? i18n.defaultLanguage
  return {
    deps: [lang],
    async search(query) {
      indexes ??= load()
      const index = (await indexes).get(lang)
      return index ? search(index, query) : []
    },
  }
}
