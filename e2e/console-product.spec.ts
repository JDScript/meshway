import { expect, test, type Page } from "@playwright/test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEnrollmentToken, getConnections, getMeshAccess, listNodes, saveConnections, setNodeCountry } from "./network-client.ts";
import { beginConnectionRuntimeOperation, getAgentConnectionsOperation, reportConnectionRuntimeOperation, reportEndpointDiscoveryOperation } from "../packages/control-client/src/generated/operations.gen.ts";
import { consoleAt, contentAlignment, executable, root, run, service, unusedPort } from "./harness.ts";

async function toggleByKeyboard(page: Page, title: string) {
  const summary = page.locator("summary").filter({ hasText: new RegExp(`^${title}$`) });
  await expect(summary).toHaveCount(1);
  await summary.focus();
  await page.keyboard.press("Enter");
}

test("console task surfaces preserve disclosures, expired copy eligibility, deep links and invalid drafts", async ({ page }) => {
  test.setTimeout(180_000);
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-console-product-"));
  const controllerPort = await unusedPort();
  const webPort = await unusedPort();
  const base = `http://127.0.0.1:${controllerPort}`;
  const webUrl = `http://127.0.0.1:${webPort}`;
  const controller = service(executable("meshway-controller"), ["serve", "--data-dir", join(temporary, "controller"), "--bind", `127.0.0.1:${controllerPort}`]);
  let web: ReturnType<typeof service> | undefined;
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await controller.ready(`${base}/api/v1/overview`, 401);
    const admin = (await readFile(join(temporary, "controller/admin.token"), "utf8")).trim();
    const grant = await createEnrollmentToken(base, admin, { expires_in_seconds: 600 });
    const state = join(temporary, "node.json");
    await run(executable("meshway-agent"), ["enroll", "--controller", base, "--token", grant.token, "--name", "product-node", "--state", state]);
    await run(executable("meshway-agent"), ["connections", "publish", "--state", state]);
    const node = (await listNodes(base, admin)).nodes[0];
    if (!node) throw new Error("Missing enrolled fixture node");
    const privateState: unknown = JSON.parse(await readFile(state, "utf8"));
    if (typeof privateState !== "object" || privateState === null || !("node_credential" in privateState) || typeof privateState.node_credential !== "string") throw new Error("Missing fixture credential");
    const credential = privateState.node_credential;
    const original = await getConnections(base, admin);
    await saveConnections(base, admin, { expected_revision: original.revision, configuration: {
      listeners: [{ id: "product-udp", node_id: node.id, transport: "native_udp", bind: "127.0.0.1:45123", advertised_endpoints: [{ host: "127.0.0.1", port: 45123 }], credential_ref: null }], candidates: [],
    } });
    const config = await getAgentConnectionsOperation(base, credential);
    const lease = await beginConnectionRuntimeOperation(base, credential, { expected_epoch: 0, instance_id: "c".repeat(64) });
    await reportConnectionRuntimeOperation(base, credential, {
      epoch: lease.epoch, sequence: 1, revision: config.payload.revision, digest: config.digest,
      state: "applied", reason: "none", listeners: [{ listener_id: "product-udp", state: "bound", reason: "none" }], peers: [],
    });
    const observedAt = new Date().toISOString();
    const expiresAt = new Date(Date.parse(observedAt) + 30_000).toISOString();
    // An authenticated report fixture, not a claim of a real STUN/network test.
    await reportEndpointDiscoveryOperation(base, credential, {
      epoch: lease.epoch, sequence: 1, revision: config.payload.revision, digest: config.digest, state: "observed",
      sockets: [{ resource_kind: "listener", resource_id: "product-udp", generation: 1, socket_id: 1, bound_address: "127.0.0.1:45123", source: "managed_udp_socket_stun",
        observations: [{ observer: "192.0.2.1:3478", outcome: "mapped", mapped_endpoint: "198.51.100.1:45123", rtt_ms: 1, observed_at: observedAt, expires_at: expiresAt }],
      }],
    });
    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: base, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    const ui = await consoleAt(webUrl, base, admin);
    await page.goto(`${webUrl}/login`);
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect.poll(() => new URL(page.url()).pathname).toBe(ui.path());

    await page.goto(`${ui.at("/connections")}?node=${node.id}`);
    await expect(page).toHaveURL(new RegExp(`${ui.path("/machines")}/${node.id}\\?tab=connections$`));
    await expect(page.getByTestId("connection-runtime")).toContainText("连接配置已应用");
    await expect(page.getByTestId("connections-editor")).toBeHidden();
    await expect(page.getByTestId("endpoint-discovery")).toBeHidden();
    await toggleByKeyboard(page, "高级手工连接配置");
    await expect(page.getByTestId("connections-editor")).toBeVisible();
    await toggleByKeyboard(page, "UDP 地址观测与当前启用帮助");
    const panel = page.getByTestId("endpoint-discovery");
    await expect(panel.getByTestId("discovery-status")).toHaveText("未验证候选观测");
    await expect(panel.getByText(/epoch/)).toBeHidden();
    await expect(panel.getByText(/generation 1/)).toBeHidden();
    await toggleByKeyboard(page, "报告技术详情");
    await expect(panel.getByText(/epoch/)).toBeVisible();
    await toggleByKeyboard(page, "报告技术详情");
    await expect(panel.getByRole("button", { name: "复制观测 IP:port", exact: true })).toBeEnabled();
    // The observer keeps ageing while its outer disclosure is closed.
    await toggleByKeyboard(page, "UDP 地址观测与当前启用帮助");
    await page.clock.install();
    await page.clock.setSystemTime(new Date(Date.parse(expiresAt) + 1));
    await page.clock.runFor(1_000);
    await toggleByKeyboard(page, "UDP 地址观测与当前启用帮助");
    await expect(panel.getByRole("button", { name: "复制历史观测 IP:port（失效）", exact: true })).toBeDisabled();
    await page.clock.setSystemTime(new Date(Date.parse(expiresAt) - 10_000));
    await page.clock.runFor(1_000);
    await expect(panel.getByRole("button", { name: "复制历史观测 IP:port（失效）", exact: true })).toBeDisabled();

    await reportConnectionRuntimeOperation(base, credential, {
      epoch: lease.epoch, sequence: 2, revision: config.payload.revision, digest: config.digest,
      state: "error", reason: "bind_failed", listeners: [], peers: [],
    });
    await page.goto(`${ui.at("/machines")}/${node.id}?tab=connections`);
    await expect(page.getByTestId("connection-runtime")).toContainText("本机监听失败");
    await expect(page.getByText("检查监听地址属于本节点、端口未被占用", { exact: false })).toBeVisible();

    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 900 });
      for (const path of ["", "/machines", `/machines/${node.id}?tab=connections`, "/machines/add", "/settings", "/dns", "/access", "/routing", "/tags", "/connections", "/topology", "/diagnostics"]) {
        await page.goto(ui.at(path));
        await expect(page.locator('[data-slot="page-body"]')).toBeVisible();
        // The page column lines up with the shell header and never scrolls sideways.
        const alignment = await contentAlignment(page);
        expect(alignment).toEqual({ left: 0, right: 0, gutter: width === 390 ? 16 : (width - 1216) / 2 + 32, overflow: false });
        if (process.env.MESHWAY_PRODUCT_SCREENSHOTS && (path === "" || path.startsWith("/machines/"))) {
          await page.screenshot({ path: test.info().outputPath(`product-${width}-${path === "" ? "overview" : "node"}.png`), fullPage: true });
        }
      }
    }
    // The old settings URL and its DNS fragment still land on the DNS diagnostics.
    await page.goto(`${webUrl}/network#dns-diagnostics`);
    await expect(page).toHaveURL(`${ui.at("/dns")}#dns-diagnostics`);
    await expect(page.getByTestId("network-dns-saved-context")).toBeVisible();
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto(`${ui.at("/machines")}/${node.id}?tab=connections`);
    await toggleByKeyboard(page, "高级手工连接配置");
    await page.getByRole("button", { name: "编辑监听器 product-udp", exact: true }).click();
    const listenerSheet = page.getByRole("dialog");
    await listenerSheet.getByLabel("监听端口", { exact: true }).fill("45124");
    await listenerSheet.getByRole("button", { name: "应用到草稿", exact: true }).click();
    await page.getByRole("button", { name: "保存连接配置", exact: true }).click();
    // Clearing dirty work must not fold the successful save feedback away.
    await expect(page.getByTestId("connections-save-status")).toBeVisible();
    await expect(page.getByTestId("connections-save-status")).toContainText("已保存");
    await page.goto(`${ui.at("/access")}`);
    const baseRevision = await page.getByTestId("access-base-revision").textContent();
    await page.getByRole("button", { name: "JSONC", exact: true }).click();
    const editor = page.getByRole("textbox", { name: "访问授权 JSONC", exact: true });
    const invalid = '// product draft comment\n{ "rules": [';
    await editor.fill(invalid);
    await expect(page.getByRole("list", { name: "JSONC 问题" })).toBeVisible();
    await page.getByRole("link", { name: "诊断", exact: true }).first().click();
    await page.getByRole("link", { name: "拓扑与流量检查", exact: true }).click();
    await expect(page).toHaveURL(/\/topology/);
    await page.getByRole("link", { name: /^访问控制/ }).first().click();
    await expect(page).toHaveURL(ui.at("/access"));
    await expect(page.getByRole("textbox", { name: "访问授权 JSONC", exact: true }).locator(".cm-line")).toHaveText(invalid.split("\n"));
    await expect(page.getByTestId("access-base-revision")).toHaveText(baseRevision ?? "");
    const savedAccess = await getMeshAccess(base, admin);
    await page.getByRole("textbox", { name: "访问授权 JSONC", exact: true }).fill(`// competing product draft\n${JSON.stringify(savedAccess.policy, null, 2)}`);
    await setNodeCountry(base, admin, { node_id: node.id, country: "US", expected_revision: savedAccess.revision });
    await page.getByTestId("access-save").click();
    await expect(page.getByTestId("access-save-status")).toContainText("冲突");
    await expect(page.getByTestId("access-base-revision")).toHaveText(baseRevision ?? "");
    await expect(page.getByRole("textbox", { name: "访问授权 JSONC", exact: true })).toContainText("competing product draft");
    await page.goto(`${ui.at("/diagnostics")}?source=${node.id}#netcheck`);
    await controller.stop();
    await page.getByRole("button", { name: "刷新观测", exact: true }).click();
    const failure = page.getByTestId("endpoint-discovery").getByRole("alert");
    await expect(failure).toContainText("端点观测读取失败");
    await expect(failure).toContainText("点击刷新观测重试");
    await expect(failure.locator("pre")).toBeHidden();
    await toggleByKeyboard(page, "错误技术详情");
    await expect(failure.locator("pre")).toBeVisible();
    expect(errors).toEqual([]);
  } finally {
    await web?.stop();
    await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
