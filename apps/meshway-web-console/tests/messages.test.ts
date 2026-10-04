import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { test } from "vitest"

// The copy is split per feature (`messages/<feature>/<locale>.json`, listed
// in `project.inlang/settings.json`) so features can grow it independently.

const project = new URL("../project.inlang/settings.json", import.meta.url)
const settings = JSON.parse(readFileSync(project, "utf8")) as {
  locales: string[]
  "plugin.inlang.messageFormat": { pathPattern: string[] }
}
const patterns = settings["plugin.inlang.messageFormat"].pathPattern

const keysOf = (pattern: string, locale: string) => {
  const file = new URL(
    `../${pattern.replace("./", "").replace("{locale}", locale)}`,
    import.meta.url
  )
  return Object.keys(
    JSON.parse(readFileSync(file, "utf8")) as Record<string, unknown>
  ).filter((key) => key !== "$schema")
}

test("every message exists in every locale, in the same file", () => {
  for (const pattern of patterns) {
    const [base, ...others] = settings.locales.map(
      (locale) => new Set(keysOf(pattern, locale))
    )
    for (const other of others)
      assert.deepEqual(
        [...(other ?? [])].sort(),
        [...(base ?? [])].sort(),
        pattern
      )
  }
})

test("no message is defined in two files", () => {
  const seen = new Map<string, string>()
  for (const pattern of patterns) {
    for (const key of keysOf(pattern, settings.locales[0] ?? "zh")) {
      assert.ok(!seen.has(key), `${key} is in ${seen.get(key)} and ${pattern}`)
      seen.set(key, pattern)
    }
  }
})
