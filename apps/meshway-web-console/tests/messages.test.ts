import assert from "node:assert/strict"
import { readdirSync, readFileSync } from "node:fs"
import { test } from "vitest"

// The copy is split by area (`messages/<area>/<locale>.json`, listed in
// `project.inlang/settings.json`); words several areas use are shared
// (`messages/<locale>.json`).

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

test("every message is used", () => {
  const root = new URL("../src/", import.meta.url)
  const source = readdirSync(root, { recursive: true, encoding: "utf8" })
    .filter((file) => /\.tsx?$/.test(file) && !file.startsWith("paraglide"))
    .map((file) => readFileSync(new URL(file, root), "utf8"))
    .join("\n")
  for (const pattern of patterns)
    for (const key of keysOf(pattern, settings.locales[0] ?? "zh"))
      assert.ok(
        new RegExp(`\\bm\\.${key}\\b`).test(source),
        `${key} is never used`
      )
})
