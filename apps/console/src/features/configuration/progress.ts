// A configuration apply's deployment, as the controller reports it: one
// progress per execution publication, each with its target nodes' receipts.
// Publications are never summed or merged here: a node can be in more than
// one, and this controller provides no per-revision totals.

export type Freshness =
  "current" | "expired" | "awaitingInstance" | "superseded"

/** What one node has reported for one publication. */
export type NodeReport = {
  saved: boolean
  applied: boolean
  failed: boolean
  freshness: Freshness
}

export type NodePhase = "failed" | "applied" | "saved" | "unreported"

/** The furthest a node got with a publication, by its own receipts. */
export function nodePhase(report: NodeReport): NodePhase {
  if (report.failed) return "failed"
  if (report.applied) return "applied"
  if (report.saved) return "saved"
  return "unreported"
}

/**
 * Nothing more will come for a publication: it was withdrawn, or every
 * target node applied it, failed it or moved on to another one.
 */
export function publicationSettled(progress: {
  withdrawnAt: string | null
  nodes: Readonly<Record<string, NodeReport>>
}): boolean {
  if (progress.withdrawnAt !== null) return true
  return Object.values(progress.nodes).every(
    (report) =>
      report.applied || report.failed || report.freshness === "superseded"
  )
}

/** What an apply's own response says about deployment. */
export type Deployment = "none" | "unpublished" | "published"

export function deploymentOf(result: {
  targetCount: number
  publicationIds: readonly string[]
}): Deployment {
  if (result.publicationIds.length > 0) return "published"
  return result.targetCount === 0 ? "none" : "unpublished"
}
