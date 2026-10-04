import type {
  ObservationPhase,
  ObservationSample,
  ObservationTask,
} from "@meshway/control-client"

// One observation subscription as this tab knows it: the controller's
// latest view of the task (`createObservation`, `getObservation`,
// `renewObservation`, the stream's `observation.status` and
// `observation.sample`) and every sample it has carried since this tab
// started following it. The controller sends only the latest sample, so the
// series is what this tab saw arrive; nothing is filled in. No I/O here.

/** A sample with the path segment it was measured on. */
export type SegmentSample = {
  /** The publication and path segment, as one key: a line never joins two. */
  segment: string
  path: readonly string[]
  sample: ObservationSample
}

export type ObservationRecord = {
  task: ObservationTask | null
  /** `observation.removed`, or the controller no longer finds it. */
  removed: boolean
  samples: SegmentSample[]
}

export const emptyRecord: ObservationRecord = {
  task: null,
  removed: false,
  samples: [],
}

/** The samples kept per subscription (a few minutes at one a second). */
export const SAMPLE_LIMIT = 600

export const isActive = (phase: ObservationPhase) =>
  phase === "preparing" || phase === "running"

const segmentOf = (task: ObservationTask) =>
  `${task.configuration.executionPublication}/${task.pathSegment ?? 0}`

/**
 * Takes the controller's view of the task. Its `latest` sample joins the
 * series once (by measurement ID; a partial window is replaced by a later
 * version of itself). A view for another subscription, or one after the
 * subscription was removed, changes nothing.
 */
export function foldTask(
  record: ObservationRecord,
  task: ObservationTask
): ObservationRecord {
  if (record.removed) return record
  if (
    record.task &&
    (record.task.subscriptionId ?? null) !== (task.subscriptionId ?? null)
  )
    return record
  let samples = record.samples
  const latest = task.latest
  if (latest && task.path && task.pathSegment != null) {
    const index = samples.findIndex(
      (entry) => entry.sample.measurementId === latest.measurementId
    )
    const entry: SegmentSample = {
      segment: segmentOf(task),
      path: task.path,
      sample: latest,
    }
    if (index === -1) {
      samples = [...samples, entry]
        .sort(
          (a, b) =>
            Date.parse(a.sample.windowStart) - Date.parse(b.sample.windowStart)
        )
        .slice(-SAMPLE_LIMIT)
    } else if (samples[index]!.sample.version < latest.version) {
      samples = samples.with(index, entry)
    }
  }
  return { task, removed: false, samples }
}

/** `observation.removed`: the subscription is gone; what it measured stays shown. */
export const removeRecord = (record: ObservationRecord): ObservationRecord =>
  record.removed ? record : { ...record, removed: true }

/** A list read (`ObservationList`) holds at most the one subscription asked for. */
export function taskFor(
  items: readonly ObservationTask[],
  subscriptionId: string
): ObservationTask | null {
  return items.find((item) => item.subscriptionId === subscriptionId) ?? null
}

/** What the record says now, at `now` (ms): a lease that ran out is ended even before the controller says so. */
export type Standing =
  | { type: "unknown" }
  | { type: "active"; phase: "preparing" | "running" }
  | { type: "lapsed" }
  | { type: "ended"; phase: ObservationPhase | null }

export function standing(record: ObservationRecord, now: number): Standing {
  const task = record.task
  if (record.removed) return { type: "ended", phase: task?.phase ?? null }
  if (!task) return { type: "unknown" }
  if (!isActive(task.phase)) return { type: "ended", phase: task.phase }
  const expires = Date.parse(task.subscriberExpiresAt ?? task.expiresAt)
  if (Number.isFinite(expires) && now >= expires) return { type: "lapsed" }
  return {
    type: "active",
    phase: task.phase === "running" ? "running" : "preparing",
  }
}
