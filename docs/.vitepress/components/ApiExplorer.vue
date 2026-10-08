<script setup lang="ts">
// The published contract rendered by RapiDoc (a dependency-light web
// component), with "try it" against a Server the reader names. The Server
// answers same-origin requests only, so a request works when the docs are
// served from the Server's origin or, in `dev:docs`, through the proxy that
// MESHWAY_CONTROLLER_URL configures.
import { useData, withBase } from "vitepress";
import { onMounted, ref, watch } from "vue";

const { isDark } = useData();
const ready = ref(false);
const storageKey = "meshway.docs.serverUrl";
const serverUrl = ref("");
const host = ref<HTMLElement | null>(null);

function apply(element: HTMLElement) {
  const dark = isDark.value;
  const attributes: Record<string, string> = {
    "spec-url": withBase("/v2/contracts/openapi.json"),
    "server-url": serverUrl.value,
    theme: dark ? "dark" : "light",
    "bg-color": dark ? "#1b1b1f" : "#ffffff",
    "text-color": dark ? "#dfdfd6" : "#3c3c43",
    "primary-color": dark ? "#64d7c9" : "#087f75",
    "nav-bg-color": dark ? "#161618" : "#f6f6f7",
    "nav-text-color": dark ? "#dfdfd6" : "#3c3c43",
    "nav-hover-bg-color": dark ? "#202127" : "#e9e9ec",
    "nav-accent-color": dark ? "#64d7c9" : "#087f75",
    "header-color": dark ? "#161618" : "#f6f6f7",
    "regular-font": "var(--vp-font-family-base)",
    "mono-font": "var(--vp-font-family-mono)",
    "font-size": "default",
    "render-style": "focused",
    "schema-style": "table",
    "schema-expand-level": "2",
    "schema-description-expanded": "true",
    "default-schema-tab": "schema",
    "response-area-height": "320px",
    "show-header": "false",
    "show-info": "true",
    "allow-spec-url-load": "false",
    "allow-spec-file-load": "false",
    "allow-spec-file-download": "true",
    "allow-server-selection": "false",
    "allow-authentication": "true",
    "allow-try": "true",
    "fill-request-fields-with-example": "true",
    "persist-auth": "false",
    "show-method-in-nav-bar": "as-colored-text",
    "use-path-in-nav-bar": "false",
    "nav-item-spacing": "compact",
    "sort-tags": "false",
    "sort-endpoints-by": "none",
    "load-fonts": "false",
  };
  for (const [name, value] of Object.entries(attributes)) element.setAttribute(name, value);
}

onMounted(async () => {
  serverUrl.value = window.localStorage.getItem(storageKey) || window.location.origin;
  await import("rapidoc");
  ready.value = true;
});

watch([ready, isDark, serverUrl, host], () => {
  if (!ready.value || !host.value) return;
  let element = host.value.querySelector("rapi-doc") as HTMLElement | null;
  if (!element) {
    element = document.createElement("rapi-doc");
    host.value.appendChild(element);
  }
  apply(element);
  window.localStorage.setItem(storageKey, serverUrl.value);
});
</script>

<template>
  <div class="explorer">
    <label class="server">
      <span>Server</span>
      <input v-model.lazy="serverUrl" type="url" spellcheck="false" placeholder="http://127.0.0.1:8420" />
      <span class="hint">同源或经 dev 代理时可直接发送请求；在 Authentication 里填入会话 token。</span>
    </label>
    <div ref="host" class="host">
      <p v-if="!ready" class="loading">正在加载契约…</p>
    </div>
  </div>
</template>

<style scoped>
.explorer {
  display: flex;
  flex-direction: column;
  height: calc(100vh - var(--vp-nav-height));
}
.server {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 24px;
  border-bottom: 1px solid var(--vp-c-divider);
  font-size: 13px;
  color: var(--vp-c-text-2);
}
.server input {
  flex: 0 1 360px;
  padding: 4px 8px;
  border: 1px solid var(--vp-c-divider);
  border-radius: 6px;
  background: var(--vp-c-bg-soft);
  color: var(--vp-c-text-1);
  font-family: var(--vp-font-family-mono);
  font-size: 13px;
}
.hint {
  color: var(--vp-c-text-3);
}
.host {
  flex: 1;
  min-height: 0;
}
.host :deep(rapi-doc) {
  height: 100%;
  width: 100%;
}
.loading {
  padding: 24px;
  color: var(--vp-c-text-2);
}
@media (max-width: 720px) {
  .server {
    flex-wrap: wrap;
  }
  .hint {
    flex-basis: 100%;
  }
}
</style>
