import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { test } from "node:test"

const css = readFileSync(new URL("./index.css", import.meta.url), "utf8")

test("no custom property refers to itself (that silently invalidates every rule using it)", () => {
  for (const [, name, value] of css.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    assert.ok(
      !value?.includes(`var(${name})`),
      `${name} is defined in terms of itself`
    )
  }
})

test("every token used by a var() is defined", () => {
  const defined = new Set(
    Array.from(css.matchAll(/(--[\w-]+)\s*:/g), ([, name]) => name)
  )
  // Supplied at runtime or by libraries rather than in this file.
  const external =
    /^--(tw-|drawer-|bleed|transform-origin|surface-|enter-|launcher-|node-color|globe-docked|edge|bar-height|panel-top|mobile-edge|commit-space|radix-|anchor-|available-|popup-|stack-|nested-|sidebar-width|tracking-)/
  for (const [, name] of css.matchAll(/var\((--[\w-]+)/g)) {
    if (!name || external.test(name)) continue
    assert.ok(defined.has(name), `${name} is used but never defined`)
  }
})
