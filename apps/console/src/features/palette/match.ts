// How the palette matches what is typed: every word typed must be found in
// one of an entry's keywords (its name first, then addresses, tags, IDs).
// Plain substrings, as the policy's pickers match: a fuzzy match would find
// almost anything inside a 64-digit hex ID.

/** 0 when `search` does not match; higher when it starts a keyword, highest on the name. */
export function matchScore(
  search: string,
  keywords: readonly string[]
): number {
  const terms = search.trim().toLowerCase().split(/\s+/).filter(Boolean)
  if (terms.length === 0) return 1
  const words = keywords.map((keyword) => keyword.toLowerCase())
  let total = 0
  for (const term of terms) {
    let best = 0
    words.forEach((word, index) => {
      const at = word.indexOf(term)
      if (at < 0) return
      const score = (at === 0 ? 2 : 1) + (index === 0 ? 1 : 0)
      best = Math.max(best, score)
    })
    if (best === 0) return 0
    total += best
  }
  return total / terms.length
}

/**
 * The entries matching `search`, best first (ties keep their order), at
 * most `limit` of them, and how many more matched.
 */
export function bestMatches<T extends { keywords: readonly string[] }>(
  entries: readonly T[],
  search: string,
  limit: number
): { shown: T[]; more: number } {
  const matched = entries
    .map((entry, index) => ({
      entry,
      index,
      score: matchScore(search, entry.keywords),
    }))
    .filter((match) => match.score > 0)
    .sort((a, b) => b.score - a.score || a.index - b.index)
  return {
    shown: matched.slice(0, limit).map((match) => match.entry),
    more: Math.max(0, matched.length - limit),
  }
}
