import assert from "node:assert/strict"
import { test } from "node:test"

import {
  globeLayout,
  MOBILE_BAR_SPACE,
  MOBILE_PAGE_HEIGHT,
  MOBILE_SHEET_BOTTOM,
  PAGE_WIDTH,
  PEEK_WIDTH,
  type Arrangement,
} from "./layout.ts"

const desktop: Arrangement = {
  mobile: false,
  page: false,
  peek: false,
  workspace: false,
  viewportHeight: 900,
}

test("the overview leaves the whole screen to the globe until a node is peeked at", () => {
  assert.deepEqual(globeLayout(desktop), {
    insets: { top: 0, right: 0, bottom: 0, left: 0 },
    recede: false,
  })
  const peek = globeLayout({ ...desktop, peek: true })
  assert.ok(peek.insets.right > PEEK_WIDTH)
  assert.equal(peek.insets.left, 0)
})

test("a page panel covers the left; a workspace covers the globe without moving it", () => {
  const page = globeLayout({ ...desktop, page: true })
  assert.ok(page.insets.left > PAGE_WIDTH)
  assert.equal(page.recede, false)
  const workspace = globeLayout({ ...desktop, page: true, workspace: true })
  assert.deepEqual(workspace.insets, page.insets)
  assert.equal(workspace.recede, true)
})

test("on small screens the globe sits above the bottom bar, and above the sheets over it", () => {
  const phone: Arrangement = { ...desktop, mobile: true, viewportHeight: 800 }
  assert.deepEqual(globeLayout(phone).insets, {
    top: 0,
    right: 0,
    bottom: MOBILE_BAR_SPACE,
    left: 0,
  })
  const page = globeLayout({ ...phone, page: true }).insets.bottom
  const peek = globeLayout({ ...phone, peek: true }).insets.bottom
  assert.equal(page, Math.round(MOBILE_PAGE_HEIGHT * 800) + MOBILE_SHEET_BOTTOM)
  assert.ok(peek > page)
})
