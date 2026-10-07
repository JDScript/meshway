<script setup lang="ts">
import { computed } from "vue";
import { format, type Point } from "./status-data";
const props = defineProps<{ points: Point[]; unit: string; label: string }>();
const values = computed(() => props.points.flatMap((p) => p.value === null ? [] : [p.value]));
const low = computed(() => Math.min(...values.value));
const high = computed(() => Math.max(...values.value));
const times = computed(() => props.points.map((p) => Date.parse(p.time)));
function x(i: number) {
  const start = Math.min(...times.value), end = Math.max(...times.value);
  return 60 + (end === start ? 0.5 : (times.value[i] - start) / (end - start)) * 520;
}
function y(value: number) {
  return 140 - (high.value === low.value ? 0.5 : (value - low.value) / (high.value - low.value)) * 110;
}
const path = computed(() => {
  let active = false;
  return props.points.map((p, i) => {
    if (p.value === null) { active = false; return ""; }
    const command = active ? "L" : "M";
    active = true;
    return `${command}${x(i)},${y(p.value)}`;
  }).join(" ");
});
</script>

<template>
  <svg v-if="values.length" viewBox="0 0 600 180" role="img" :aria-label="`${label} (${unit})`" class="chart">
    <title>{{ label }} · {{ unit }}；橙色为相对上一有效 main 样本变化超过 10%</title>
    <path d="M60 20V145H585" class="axis" />
    <text x="55" y="30" text-anchor="end">{{ format(high) }}</text>
    <text x="55" y="142" text-anchor="end">{{ format(low) }}</text>
    <path :d="path" class="line" />
    <template v-for="(point, i) in points" :key="i">
      <a v-if="point.value !== null" :href="point.url" target="_blank" rel="noreferrer">
        <circle :cx="x(i)" :cy="y(point.value)" r="5" :class="{ changed: point.delta !== null && Math.abs(point.delta) > 10 }">
          <title>{{ point.title }} · {{ format(point.value) }} {{ unit }} · Δ {{ format(point.delta) }}%</title>
        </circle>
      </a>
    </template>
    <text x="60" y="168">{{ points[0]?.time.slice(0, 10) }}</text>
    <text x="580" y="168" text-anchor="end">{{ points.at(-1)?.time.slice(0, 10) }}</text>
  </svg>
  <p v-else class="empty">此序列没有有效测量值。</p>
</template>

<style scoped>
.chart { width: 100%; max-height: 240px; overflow: visible; }
.axis { stroke: var(--vp-c-divider); fill: none; }
.line { stroke: var(--vp-c-brand-1); stroke-width: 2; fill: none; }
circle { fill: var(--vp-c-brand-1); }
circle.changed { fill: #dc8613; stroke: var(--vp-c-text-1); }
text { font-size: 11px; fill: var(--vp-c-text-2); }
</style>
