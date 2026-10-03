import assert from "node:assert/strict"
import { test } from "node:test"

import {
  afterApply,
  discardEdits,
  editText,
  isDirty,
  isOutdated,
  loadSource,
  needsSource,
  withChecked,
} from "./draft-state.ts"

const SOURCE = { revision: 3, text: '{ "dns": { "domain": "a" } }' }

test("a read source becomes a draft without edits", () => {
  const entry = loadSource(null, SOURCE)
  assert.equal(entry.baseRevision, 3)
  assert.equal(entry.input.text, SOURCE.text)
  assert.equal(isDirty(entry), false)
  assert.equal(needsSource(entry, 3), false)
  // Behind the controller with nothing typed: read again.
  assert.equal(needsSource(entry, 4), true)
  assert.equal(needsSource(null, undefined), true)
})

test("any text is kept as typed, invalid or with comments", () => {
  const entry = loadSource(null, SOURCE)
  const invalid = editText(entry, '// note\n{ "dns": ')
  assert.equal(invalid.input.text, '// note\n{ "dns": ')
  assert.equal(isDirty(invalid), true)
  // Typing it back is no draft.
  assert.equal(isDirty(editText(invalid, SOURCE.text)), false)
})

test("a newer revision never moves a draft with edits", () => {
  const edited = editText(loadSource(null, SOURCE), "// mine\n" + SOURCE.text)
  const after = loadSource(edited, { revision: 5, text: "{}" })
  assert.equal(after, edited)
  assert.equal(after.baseRevision, 3)
  assert.equal(needsSource(after, 5), false)
  assert.equal(isOutdated(after, 5), true)
  assert.equal(isOutdated(after, 3), false)
  // Without edits it simply follows the controller.
  const clean = loadSource(loadSource(null, SOURCE), {
    revision: 5,
    text: "{}",
  })
  assert.equal(clean.baseRevision, 5)
  assert.equal(isOutdated(clean, 6), false)
})

test("the same revision read again keeps what was checked", () => {
  const entry = withChecked(loadSource(null, SOURCE), {
    text: SOURCE.text,
    issues: [{ path: "/dns", message: "x" }],
    requirements: [],
  })
  assert.equal(loadSource(entry, SOURCE), entry)
})

test("discarding returns to the source and forgets the check", () => {
  const edited = withChecked(editText(loadSource(null, SOURCE), "{"), {
    text: "{",
    issues: [{ path: "", message: "x" }],
    requirements: [],
  })
  const discarded = discardEdits(edited)
  assert.equal(discarded.input.text, SOURCE.text)
  assert.equal(discarded.checked, null)
  assert.equal(isDirty(discarded), false)
})

test("a check is kept beside later edits, with its own text", () => {
  const entry = withChecked(editText(loadSource(null, SOURCE), "{ }"), {
    text: "{ }",
    issues: [{ path: "/dns", message: "required" }],
    requirements: [],
  })
  const later = editText(entry, "{  }")
  assert.equal(later.checked?.text, "{ }")
})

test("applying ends the draft that holds the applied text", () => {
  const edited = editText(loadSource(null, SOURCE), "// x\n" + SOURCE.text)
  assert.equal(afterApply(edited, "// x\n" + SOURCE.text), null)
  assert.equal(afterApply(edited, "other"), edited)
  assert.equal(afterApply(null, "x"), null)
})
