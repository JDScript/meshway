<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { withBase } from "vitepress";
import StatusChart from "./StatusChart.vue";
import { change, format, parseIndex, type Point, type Run, type StatusIndex } from "./status-data";

const index = ref<StatusIndex | null>(null);
const loading = ref(true);
const error = ref("");
const limit = ref(20);
const metricLabel = ref("");
onMounted(async () => {
  try {
    const response = await fetch(withBase("/status/index.json"), { cache: "no-cache" });
    if (response.status === 404) return;
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const data: unknown = await response.json();
    index.value = parseIndex(data);
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally { loading.value = false; }
});
const runs = computed(() => index.value?.runs ?? []);
const recent = computed(() => runs.value.slice(0, limit.value));
const main = computed(() => runs.value.filter((r) => r.branch === "main").slice().reverse());
const labels = computed(() => [...new Set(main.value.flatMap((r) => r.artifacts.flatMap((a) =>
  a.reports.flatMap((p) => p.metrics.map((m) => m.label)))))].sort());
const selectedLabel = computed(() => metricLabel.value || labels.value[0] || "");
const charts = computed(() => {
  const groups = new Map<string, { platform: string; scope: string; unit: string; points: Point[]; previous: number | null }>();
  for (const run of main.value) for (const a of run.artifacts) for (const report of a.reports) {
    // Existing workflows can upload old files left in a runner workspace.
    if (report.revision !== run.commit || !a.runner) continue;
    for (const m of report.metrics) {
      if (m.label !== selectedLabel.value) continue;
      const key = `${a.runner}/${m.label}/${m.scope}`;
      const group = groups.get(key) ?? { platform: a.runner, scope: m.scope, unit: m.unit, points: [], previous: null };
      const eligible = !report.quick && report.dirty === false && report.sourcesUnchanged !== false && m.samples === m.attempted;
      const delta = eligible ? change(m.value, group.previous) : null;
      group.points.push({ value: m.value, time: run.createdAt, url: a.url, delta,
        title: `run ${run.id} · ${run.commit.slice(0, 8)} · ${report.reportId} · ${m.samples}/${m.attempted} samples · load ${format(report.loadMin)}–${format(report.loadMax)}${eligible ? "" : " · diagnostic/incomplete"}` });
      if (eligible && m.value !== null) group.previous = m.value;
      groups.set(key, group);
    }
  }
  return [...groups.entries()].map(([key, group]) => ({ key, ...group, points: group.points.slice(-limit.value) }));
});
const matrixRuns = computed(() => main.value.filter((r) => r.workflow === "Validate").slice(-limit.value).reverse());
const rows = computed(() => [...new Set(matrixRuns.value.flatMap((r) => r.artifacts.flatMap((a) =>
  a.matrices.flatMap((m) => m.rows.map((row) => row.row)))))].sort());
function cells(run: Run, row: string) {
  return run.artifacts.flatMap((a) => a.matrices.flatMap((m) => m.rows.filter((r) => r.row === row).map((r) => ({
    label: `${m.short ? "短测 " : ""}${r.passed === null ? "未知" : r.passed ? "通过" : "失败"}`,
    title: `${r.reportId ?? m.reportId} · ${format(r.seconds)} s · load ${r.load ?? "未测"} · ${a.runner ?? "runner 未知"}`,
    passed: r.passed, short: m.short, url: a.url,
  }))));
}
const builds = computed(() => {
  const groups = new Map<string, { target: string; binary: string; runner: string; points: Point[]; sha: string;
    seconds: number | null; run: Run; previous: number | null }>();
  for (const run of main.value) for (const a of run.artifacts) for (const b of a.builds) {
    const runner = a.runner ?? "runner 未知";
    const key = `${runner}/${b.target}/${b.binary}`;
    const group = groups.get(key) ?? { target: b.target, binary: b.binary, runner, points: [],
      sha: b.sha256, seconds: a.seconds, run, previous: null };
    group.points.push({ value: b.bytes / 1024 ** 2, time: run.createdAt, url: a.url,
      delta: change(b.bytes, group.previous), title: `run ${run.id} · ${run.commit.slice(0, 8)} · ${b.bytes} bytes` });
    Object.assign(group, { sha: b.sha256, seconds: a.seconds, run, previous: b.bytes });
    groups.set(key, group);
  }
  return [...groups.entries()].map(([key, group]) => ({ key, ...group, points: group.points.slice(-limit.value) }));
});
function verdict(value: string | null) { return value === "success" ? "pass" : value === "failure" ? "fail" : ""; }
</script>

<template>
  <section class="status-page" aria-label="CI 与性能状态">
    <p>数据来源：GitHub Actions runs／jobs／artifacts 与报告摘要；仓库 {{ index?.repository ?? "JDScript/meshway" }}。<br>
      索引更新时间：{{ index?.updatedAt ?? "尚未同步" }}。索引为本地静态快照，artifact 链接需要 GitHub 仓库权限。</p>
    <p v-if="loading" role="status">正在读取状态索引…</p>
    <p v-else-if="error" role="alert" class="empty">索引读取失败：{{ error }}。请运行 <code>mise run status:sync</code> 更新兼容版本。</p>
    <p v-else-if="!runs.length" class="empty">暂无 CI 运行数据。请运行 <code>mise run status:sync</code>；新 workflow 完成并发布 ci-results 后即可显示。缺测不会显示为零或通过。</p>
    <label>显示最近 <select v-model.number="limit"><option :value="10">10</option><option :value="20">20</option><option :value="50">50</option><option :value="200">200</option></select> 次</label>

    <h2 id="recent-runs">最近运行</h2>
    <div v-if="recent.length" class="scroll"><table>
      <thead><tr><th>Workflow／触发</th><th>分支／commit</th><th>Jobs · runner · 耗时 · 结论</th><th>Artifacts</th></tr></thead>
      <tbody><tr v-for="run in recent" :key="run.id">
        <td><a :href="run.url">{{ run.workflow }} #{{ run.id }}</a><br>{{ run.event }} · attempt {{ run.attempt }}<br>
          <span :class="verdict(run.conclusion)">{{ run.conclusion ?? run.status }}</span><br><small>{{ run.createdAt }}</small></td>
        <td>{{ run.branch ?? "未知分支" }}<br><code>{{ run.commit.slice(0, 8) }}</code><br>{{ run.message || "无 commit message" }}</td>
        <td><div v-for="job in run.jobs" :key="job.id" class="entry" :title="`${job.startedAt ?? '—'} → ${job.completedAt ?? '—'}`">
          <a :href="job.url">{{ job.name }}</a><br>{{ job.runner ?? "runner 未分配" }} · {{ format(job.seconds) }} s ·
          <span :class="verdict(job.conclusion)">{{ job.conclusion ?? job.status }}</span>
        </div><span v-if="!run.jobs.length">尚无 job</span></td>
        <td><div v-for="a in run.artifacts" :key="a.id" class="entry"><a :href="a.url">{{ a.name }}</a><br>
          {{ format(a.bytes / 1024 ** 2) }} MiB · {{ a.expired ? "已过期／删除" : "可下载" }}
          <span v-if="a.state === 'error'" class="fail"><br>{{ a.error }}</span>
        </div><span v-if="!run.artifacts.length">无 artifact</span></td>
      </tr></tbody>
    </table></div><p v-else class="empty">尚无运行记录。</p>

    <h2 id="performance-series">性能时间序列</h2>
    <p>main · profile off · 按 runner 与参数分组。橙点表示相对上一有效 main 结果变化 &gt;10%（双向）；hover／键盘聚焦查看 run、commit、报告与 load，点击打开 artifact。短测、dirty 或不完整样本保留显示但不作变化基线；不同源码的残留报告不进入曲线。</p>
    <label v-if="labels.length">指标 <select v-model="metricLabel"><option value="">{{ labels[0] }}</option><option v-for="label in labels.slice(1)" :key="label" :value="label">{{ label }}</option></select></label>
    <div v-for="chart in charts" :key="chart.key" class="card">
      <h3>{{ chart.platform }} · {{ selectedLabel }} ({{ chart.unit }})</h3>
      <details><summary>测量参数</summary><code>{{ chart.scope }}</code></details>
      <StatusChart :points="chart.points" :unit="chart.unit" :label="`${chart.platform} ${selectedLabel}`" />
      <p>最新：{{ format(chart.points.at(-1)?.value ?? null) }} {{ chart.unit }} · Δ {{ format(chart.points.at(-1)?.delta ?? null) }}%</p>
    </div><p v-if="!charts.length" class="empty">尚无可归属 runner 与当前 commit 的性能报告。</p>

    <h2 id="validation-matrix">验收矩阵</h2>
    <p>最近 main Validate：行 × 运行。短测单独标注，不构成验收；“—”表示没有该行报告。</p>
    <div v-if="rows.length" class="scroll"><table>
      <thead><tr><th>矩阵行</th><th v-for="run in matrixRuns" :key="run.id"><a :href="run.url">{{ run.commit.slice(0, 8) }}</a><br><small>#{{ run.id }}</small></th></tr></thead>
      <tbody><tr v-for="row in rows" :key="row"><th>{{ row }}</th><td v-for="run in matrixRuns" :key="run.id">
        <div v-for="(cell, i) in cells(run, row)" :key="i"><a :href="cell.url" :title="cell.title" :class="{ pass: cell.passed === true && !cell.short, fail: cell.passed === false }">{{ cell.label }}</a><br><small>{{ cell.title }}</small></div>
        <span v-if="!cells(run, row).length">—</span>
      </td></tr></tbody>
    </table></div><p v-else class="empty">尚无 matrix.json 行结果。</p>

    <h2 id="build-artifacts">构建产物</h2>
    <p>main · 按 target／程序／编译机分组；大小为解压后的二进制字节数，SHA-256 从实际文件计算。耗时为整个构建 job 墙钟。</p>
    <div v-if="builds.length" class="scroll"><table>
      <thead><tr><th>Target／程序</th><th>体积趋势 (MiB)</th><th>最新 SHA-256</th><th>编译机／耗时／commit</th></tr></thead>
      <tbody><tr v-for="b in builds" :key="b.key"><td>{{ b.target }}<br>{{ b.binary }}</td>
        <td class="trend">{{ format(b.points.at(-1)?.value ?? null) }} MiB<StatusChart :points="b.points" unit="MiB" :label="`${b.target} ${b.binary}`" /></td>
        <td><code class="hash">{{ b.sha }}</code></td><td>{{ b.runner }}<br>{{ format(b.seconds) }} s<br><a :href="b.run.url">{{ b.run.commit.slice(0, 8) }}</a></td>
      </tr></tbody>
    </table></div><p v-else class="empty">尚无可下载并核验的 Node 构建产物。</p>
  </section>
</template>

<style scoped>
.status-page { font-size: 14px; }
.scroll { overflow-x: auto; }
.scroll table { display: table; width: 100%; font-size: 13px; }
td { vertical-align: top; min-width: 130px; }
.entry + .entry { margin-top: 12px; }
.empty, .card { padding: 16px; border: 1px solid var(--vp-c-divider); border-radius: 8px; margin: 16px 0; }
.empty { color: var(--vp-c-text-2); background: var(--vp-c-bg-soft); }
.pass { color: var(--vp-c-green-1); }
.fail { color: var(--vp-c-red-1); }
select { border: 1px solid var(--vp-c-divider); border-radius: 4px; padding: 4px 8px; max-width: 100%; }
small { color: var(--vp-c-text-2); }
.hash, details code { overflow-wrap: anywhere; white-space: normal; }
.trend { min-width: 240px; }
</style>
