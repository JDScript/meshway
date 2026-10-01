import {
  parseCountryList,
  validateDraft,
  type ActionKind,
  type DefaultAction,
  type DestinationScope,
  type LocalIssue,
  type RuleDraft,
  type SelectorDraft,
} from "./policy-model.ts"

// Focused rule-builder state. It widens a RuleDraft with explicit "any" modes
// so that matching everything is a deliberate choice rather than the silent
// result of leaving fields empty. `toRuleDraft` narrows it back; nothing here
// reaches the controller without going through `toPolicy`.

/** One opening of the rule builder. */
export type BuilderSession = {
  /** Increments per opening so the form remounts with fresh state. */
  id: number
  mode: "edit" | "new"
  /** What the rule was when the builder opened; unapplied edits compare against it. */
  initial: RuleForm
  /** Position the rule has (edit) or will take (new) in the ordered list. */
  position: number
  /** Unapplied form contents to resume with (restored after navigating away). */
  working?: RuleForm
}

/** An open builder holding edits that are not applied to the draft yet. */
export type WorkingRule = { session: BuilderSession; form: RuleForm }

export type SourceMode = "any" | "specific"
export type TargetMode = "all" | "specific"

export type RuleForm = {
  key: string
  id: string
  sourceMode: SourceMode
  sourceNodeIds: string[]
  sourceCountries: string
  scope: DestinationScope
  targetMode: TargetMode
  destinations: SelectorDraft[]
  action: ActionKind
  /**
   * Action the rule had when the form opened. Only an explicit switch from
   * route to reject clears via/exit; a stored reject that still names hops
   * (reported by the compiler) is preserved until the user edits them.
   */
  initialAction: ActionKind
  /** Kept while the form shows "reject" so switching back restores the path. */
  via: string[]
  exitNodeId: string
}

export function toRuleForm(rule: RuleDraft): RuleForm {
  const anySource =
    rule.sourceNodeIds.length === 0 && parseCountryList(rule.sourceCountries).length === 0
  return {
    key: rule.key,
    id: rule.id,
    sourceMode: anySource ? "any" : "specific",
    sourceNodeIds: [...rule.sourceNodeIds],
    sourceCountries: rule.sourceCountries,
    scope: rule.scope,
    targetMode: rule.destinations.length === 0 ? "all" : "specific",
    destinations: rule.destinations.map((selector) => ({ ...selector })),
    action: rule.action,
    initialAction: rule.action,
    via: [...rule.via],
    exitNodeId: rule.exitNodeId,
  }
}

/** New rules start narrow: source and target must be chosen or made "any". */
export function newRuleForm(key: string, id: string): RuleForm {
  return {
    key,
    id,
    sourceMode: "specific",
    sourceNodeIds: [],
    sourceCountries: "",
    scope: "internet",
    targetMode: "specific",
    destinations: [{ key: `${key}-d0`, kind: "domain_suffix", value: "" }],
    action: "route",
    initialAction: "route",
    via: [],
    exitNodeId: "",
  }
}

export function toRuleDraft(form: RuleForm): RuleDraft {
  const specificSource = form.sourceMode === "specific"
  const keepPath = form.action === "route" || form.initialAction === "reject"
  return {
    key: form.key,
    id: form.id,
    sourceNodeIds: specificSource ? [...form.sourceNodeIds] : [],
    sourceCountries: specificSource ? form.sourceCountries : "",
    scope: form.scope,
    destinations:
      form.targetMode === "specific"
        ? form.destinations.map(({ key, kind, value }) => ({ key, kind, value }))
        : [],
    action: form.action,
    // A reject never names via or exit nodes once the user switches to it.
    via: keepPath ? [...form.via] : [],
    exitNodeId: keepPath ? form.exitNodeId : "",
  }
}

export function sameForm(a: RuleForm, b: RuleForm): boolean {
  return JSON.stringify(toRuleDraft(a)) === JSON.stringify(toRuleDraft(b))
}

/**
 * Issues that block applying the form to the draft: everything the policy
 * model checks for this rule (including ID uniqueness against the others),
 * plus explicit-mode choices left incomplete.
 */
export function validateRuleForm(
  form: RuleForm,
  others: RuleDraft[],
  defaultAction: DefaultAction
): LocalIssue[] {
  const candidate = toRuleDraft(form)
  const issues = validateDraft({ defaultAction, rules: [...others, candidate] }).filter(
    (issue) => issue.ruleKey === form.key
  )
  if (
    form.sourceMode === "specific" &&
    form.sourceNodeIds.length === 0 &&
    parseCountryList(form.sourceCountries).length === 0
  ) {
    issues.push({
      ruleKey: form.key,
      field: "source",
      message: "选择来源节点或填写国家，或改为「任意来源」。",
    })
  }
  if (form.targetMode === "specific" && form.destinations.length === 0) {
    issues.push({
      ruleKey: form.key,
      field: "destination_any",
      message: "添加目标条件，或改为「范围内全部目标」。",
    })
  }
  return issues
}
