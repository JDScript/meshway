import assert from "node:assert/strict"
import type { ConfigurationView } from "@meshway/control-client"
import { test } from "vitest"

import {
  afterApply,
  checkedNow,
  discardEdits,
  editPart,
  editText,
  holdsDocument,
  holdsOperations,
  isDirty,
  isOutdated,
  loadSource,
  needsSource,
  operationsOf,
  withChecked,
  type DraftEntry,
} from "@/lib/configuration/draft-state"

const SOURCE = { revision: 3, text: '{ "dns": { "domain": "a" } }' }

const VIEW: ConfigurationView = {
  access: [{ remark: "all", src: ["*"], dst: ["*"], ip: ["*"] }],
  addressPools: { ipv4: ["100.64.0.0/10"], ipv6: [] },
  defaults: { access: "deny", topology: "allow", dialers: {} },
  dns: { domain: "a.example" },
  groups: {},
  nodes: {},
  routing: {},
  ruleSets: {},
  tagsOwners: {},
  topology: [],
}
const AT_3 = { revision: 3, configuration: VIEW }
const RULE = { src: ["*"], dst: ["tag:web"], ip: ["tcp:443"] }

const textOf = (entry: DraftEntry) =>
  entry.input.kind === "document" ? entry.input.text : null

test("a read source becomes a draft without edits", () => {
  const entry = loadSource(null, SOURCE)
  assert.equal(entry.baseRevision, 3)
  assert.equal(textOf(entry), SOURCE.text)
  assert.equal(isDirty(entry), false)
  assert.equal(needsSource(entry, 3), false)
  // Behind the controller with nothing typed: read again.
  assert.equal(needsSource(entry, 4), true)
  assert.equal(needsSource(null, undefined), true)
})

test("any text is kept as typed, invalid or with comments", () => {
  const entry = loadSource(null, SOURCE)
  const invalid = editText(entry, '// note\n{ "dns": ')
  assert.equal(textOf(invalid), '// note\n{ "dns": ')
  assert.equal(isDirty(invalid), true)
  assert.equal(holdsDocument(invalid), true)
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
    input: { kind: "document", text: SOURCE.text },
    issues: [{ path: "/dns", message: "x" }],
    requirements: [],
  })
  assert.equal(loadSource(entry, SOURCE), entry)
})

test("discarding returns to the source and forgets the check", () => {
  const edited = withChecked(editText(loadSource(null, SOURCE), "{"), {
    input: { kind: "document", text: "{" },
    issues: [{ path: "", message: "x" }],
    requirements: [],
  })
  const discarded = discardEdits(edited)
  assert.ok(discarded)
  assert.equal(textOf(discarded), SOURCE.text)
  assert.equal(discarded.checked, null)
  assert.equal(isDirty(discarded), false)
})

test("a check is kept beside later edits, and only counts for its own input", () => {
  const entry = withChecked(editText(loadSource(null, SOURCE), "{ }"), {
    input: { kind: "document", text: "{ }" },
    issues: [{ path: "/dns", message: "required" }],
    requirements: [],
  })
  assert.equal(checkedNow(entry), entry.checked)
  const later = editText(entry, "{  }")
  assert.equal(later.checked, entry.checked)
  assert.equal(checkedNow(later), null)
})

test("applying ends the draft that holds the applied input", () => {
  const edited = editText(loadSource(null, SOURCE), "// x\n" + SOURCE.text)
  assert.equal(
    afterApply(edited, { kind: "document", text: "// x\n" + SOURCE.text }),
    null
  )
  assert.equal(afterApply(edited, { kind: "document", text: "other" }), edited)
  assert.equal(afterApply(null, { kind: "document", text: "x" }), null)
})

test("a form edit starts an operations draft at the view's revision", () => {
  const entry = editPart(null, AT_3, "access", [RULE])
  assert.ok(entry)
  assert.equal(entry.baseRevision, 3)
  assert.equal(entry.baseText, null)
  assert.equal(holdsOperations(entry), true)
  assert.deepEqual(operationsOf(entry), [
    { type: "replaceAccess", access: [RULE] },
  ])
  // The source was never read: opening it reads it, which keeps the draft.
  assert.equal(needsSource(entry, 3), false)
  assert.equal(loadSource(entry, SOURCE), entry)
})

test("edits to one part collapse into one operation; back to the base drops it", () => {
  const first = editPart(null, AT_3, "access", [RULE])
  const second = editPart(first, AT_3, "access", [RULE, RULE])
  assert.equal(operationsOf(second).length, 1)
  const back = editPart(second, AT_3, "access", VIEW.access)
  assert.equal(back, null)
})

test("back to the base over a read source leaves the source draft without edits", () => {
  const read = loadSource(null, SOURCE)
  const edited = editPart(read, AT_3, "access", [RULE])
  assert.ok(edited)
  assert.equal(edited.baseText, SOURCE.text)
  const back = editPart(edited, AT_3, "access", VIEW.access)
  assert.ok(back)
  assert.equal(textOf(back), SOURCE.text)
  assert.equal(isDirty(back), false)
})

test("a source read at another revision is not the forms' base", () => {
  const read = loadSource(null, { revision: 2, text: "{}" })
  const edited = editPart(read, AT_3, "access", [RULE])
  assert.equal(edited?.baseRevision, 3)
  assert.equal(edited?.baseText, null)
})

test("forms refuse over source edits, and the source over form edits", () => {
  const typed = editText(loadSource(null, SOURCE), "// mine")
  assert.equal(editPart(typed, AT_3, "access", [RULE]), typed)
  const formed = editPart(null, AT_3, "access", [RULE])
  assert.ok(formed)
  assert.equal(editText(formed, "{}"), formed)
})

test("an operations draft keeps its base when the controller moves on", () => {
  const formed = editPart(null, AT_3, "access", [RULE])
  const moved = { revision: 4, configuration: { ...VIEW, access: [] } }
  const next = editPart(formed, moved, "defaults", {
    topology: "deny",
    dialers: {},
  })
  assert.ok(next)
  assert.equal(next.baseRevision, 3)
  assert.equal(next.input.kind === "operations" && next.input.base, VIEW)
  assert.equal(isOutdated(next, 4), true)
  assert.deepEqual(
    operationsOf(next).map((op) => op.type),
    ["replaceAccess", "replaceDefaults"]
  )
})

test("discarding form edits without a read source ends the draft", () => {
  const formed = editPart(null, AT_3, "access", [RULE])
  assert.ok(formed)
  assert.equal(discardEdits(formed), null)
})

test("applying operations ends the draft holding the same operations", () => {
  const formed = editPart(null, AT_3, "access", [RULE])
  assert.ok(formed)
  assert.equal(afterApply(formed, formed.input), null)
  const more = editPart(formed, AT_3, "access", [RULE, RULE])
  assert.equal(afterApply(more, formed.input), more)
})
