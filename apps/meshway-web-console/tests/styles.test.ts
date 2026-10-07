import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { test } from "vitest"

import { cn } from "@/lib/utils"

const css = readFileSync(new URL("../src/index.css", import.meta.url), "utf8")

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

test("class merging knows every custom type size and radius of the theme", () => {
  const theme = css.slice(css.indexOf("@theme"))
  const named = (prefix: string) =>
    Array.from(
      theme.matchAll(new RegExp(`--${prefix}-([a-z]+):`, "g")),
      ([, name]) => name
    )
  const tailwindRadii = new Set([
    "xs",
    "sm",
    "md",
    "lg",
    "xl",
    "2xl",
    "3xl",
    "4xl",
  ])
  // A size it did not know would count as a colour and drop the colour.
  for (const name of named("text")) {
    assert.equal(
      cn("text-muted-foreground", `text-${name}`),
      `text-muted-foreground text-${name}`
    )
    assert.equal(cn("text-sm", `text-${name}`), `text-${name}`)
  }
  for (const name of named("radius").filter(
    (name) => !tailwindRadii.has(name ?? "")
  ))
    assert.equal(cn("rounded-lg", `rounded-${name}`), `rounded-${name}`)
})
