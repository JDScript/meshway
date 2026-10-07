// Runtime boundary for the versioned, locally synced Actions summary index.
function object(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("Expected object");
  return value as Record<string, unknown>;
}
function text(value: unknown): string {
  if (typeof value !== "string") throw new Error("Expected string");
  return value;
}
function number(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) throw new Error("Expected finite number");
  return value;
}
function boolean(value: unknown): boolean {
  if (typeof value !== "boolean") throw new Error("Expected boolean");
  return value;
}
function nullable<T>(value: unknown, parse: (value: unknown) => T): T | null {
  return value === null ? null : parse(value);
}
function list<T>(value: unknown, parse: (value: unknown) => T): T[] {
  if (!Array.isArray(value)) throw new Error("Expected array");
  return value.map(parse);
}
function url(value: unknown): string {
  const result = text(value);
  if (!result.startsWith("https://github.com/")) throw new Error("Expected GitHub URL");
  return result;
}
function metric(value: unknown) {
  const v = object(value);
  return { label: text(v.label), unit: text(v.unit), scope: text(v.scope), value: nullable(v.value, number),
    samples: number(v.samples), attempted: number(v.attempted) };
}
function report(value: unknown) {
  const v = object(value);
  return { reportId: text(v.reportId), kind: text(v.kind), revision: nullable(v.revision, text),
    dirty: nullable(v.dirty, boolean), quick: boolean(v.quick), sourcesUnchanged: nullable(v.sourcesUnchanged, boolean),
    loadMin: nullable(v.loadMin, number), loadMax: nullable(v.loadMax, number), metrics: list(v.metrics, metric) };
}
function matrixRow(value: unknown) {
  const v = object(value);
  return { row: text(v.row), passed: nullable(v.passed, boolean), reportId: nullable(v.reportId, text),
    seconds: nullable(v.seconds, number), load: nullable(v.load, text) };
}
function matrix(value: unknown) {
  const v = object(value);
  return { reportId: text(v.reportId), short: boolean(v.short), rows: list(v.rows, matrixRow) };
}
function build(value: unknown) {
  const v = object(value);
  return { target: text(v.target), binary: text(v.binary), bytes: number(v.bytes), sha256: text(v.sha256) };
}
function job(value: unknown) {
  const v = object(value);
  return { id: number(v.id), name: text(v.name), runner: nullable(v.runner, text), status: text(v.status),
    conclusion: nullable(v.conclusion, text), startedAt: nullable(v.startedAt, text),
    completedAt: nullable(v.completedAt, text), seconds: nullable(v.seconds, number), url: url(v.url) };
}
function artifact(value: unknown) {
  const v = object(value);
  return { id: number(v.id), name: text(v.name), bytes: number(v.bytes), expired: boolean(v.expired), url: url(v.url),
    jobId: nullable(v.jobId, number), runner: nullable(v.runner, text), seconds: nullable(v.seconds, number),
    state: text(v.state), error: nullable(v.error, text), reports: list(v.reports, report),
    matrices: list(v.matrices, matrix), builds: list(v.builds, build) };
}
function run(value: unknown) {
  const v = object(value);
  return { id: number(v.id), attempt: number(v.attempt), workflow: text(v.workflow), event: text(v.event),
    branch: nullable(v.branch, text), commit: text(v.commit), message: text(v.message), createdAt: text(v.createdAt),
    updatedAt: text(v.updatedAt), status: text(v.status), conclusion: nullable(v.conclusion, text), url: url(v.url),
    jobs: list(v.jobs, job), artifacts: list(v.artifacts, artifact) };
}
export function parseIndex(value: unknown) {
  const v = object(value);
  if (v.schemaVersion !== 1) throw new Error("Unsupported status schemaVersion");
  return { schemaVersion: 1, repository: text(v.repository), updatedAt: nullable(v.updatedAt, text), runs: list(v.runs, run) };
}
export type StatusIndex = ReturnType<typeof parseIndex>;
export type Run = ReturnType<typeof run>;
export type Point = { value: number | null; time: string; title: string; url: string; delta: number | null };
export function change(value: number | null, previous: number | null): number | null {
  return value !== null && previous !== null && previous !== 0 ? (value / previous - 1) * 100 : null;
}
export function format(value: number | null, digits = 2): string {
  return value === null ? "未测" : value.toLocaleString("zh-CN", { maximumFractionDigits: digits });
}
