import { expect, test, type Locator, type Page } from "@playwright/test";
import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEnrollmentToken, getConnections, listNodes, revokeNode, saveConnections, setNodeCountry } from "./network-client.ts";
import { executable, expandDetails, root, run, service, unusedPort } from "./harness.ts";
import { revealAdvanced } from "./web-actions.ts";

async function choose(page: Page, trigger: Locator, name: RegExp) {
  await trigger.click();
  await expect(trigger).toHaveAttribute("aria-expanded", "true");
  const popup = await trigger.getAttribute("aria-controls");
  if (!popup) throw new Error("Select did not identify its listbox");
  await page.locator(`[id=${JSON.stringify(popup)}]`).getByRole("option", { name }).click();
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
}

async function secret(path: string, field: "private_key" | "node_credential") {
  const value: unknown = JSON.parse(await readFile(path, "utf8"));
  if (typeof value !== "object" || value === null) throw new Error("Invalid private fixture file");
  if (field === "private_key" && "private_key" in value && typeof value.private_key === "string") return value.private_key;
  if (field === "node_credential" && "node_credential" in value && typeof value.node_credential === "string") return value.node_credential;
  throw new Error("Private fixture field is missing");
}

test("connection identities, reverse-dial intent and independent receipts cross the real stack", async ({ page, browser }) => {
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-connections-e2e-"));
  const controllerPort = await unusedPort();
  const webPort = await unusedPort();
  const controllerUrl = `http://127.0.0.1:${controllerPort}`;
  const webUrl = `http://127.0.0.1:${webPort}`;
  const controller = service(executable("meshway-controller"), ["serve", "--data-dir", join(temporary, "controller"), "--bind", `127.0.0.1:${controllerPort}`]);
  let web: ReturnType<typeof service> | undefined;
  const anonymous = await browser.newContext();
  const browserErrors: string[] = [];
  page.on("pageerror", error => browserErrors.push(error.message));
  try {
    await controller.ready(`${controllerUrl}/api/v1/overview`, 401);
    const admin = (await readFile(join(temporary, "controller/admin.token"), "utf8")).trim();
    const secrets = [admin];
    for (const name of ["nyc", "lax", "sha", "pending-key"]) {
      const grant = await createEnrollmentToken(controllerUrl, admin, { expires_in_seconds: 600 });
      const state = join(temporary, `${name}.json`);
      await run(executable("meshway-agent"), ["enroll", "--controller", controllerUrl, "--token", grant.token, "--name", name, "--state", state]);
      secrets.push(await secret(state, "node_credential"));
      if (name === "pending-key") continue;
      const identity = await readFile(state);
      await run(executable("meshway-agent"), ["connections", "publish", "--state", state]);
      expect((await readFile(state)).equals(identity), "key publication preserves the enrollment file bytes").toBe(true);
      expect((await stat(`${state}.connection-key.json`)).mode & 0o777).toBe(0o600);
      secrets.push(await secret(`${state}.connection-key.json`, "private_key"));
    }
    const nodes = (await listNodes(controllerUrl, admin)).nodes;
    const nyc = nodes.find(node => node.name === "nyc");
    const lax = nodes.find(node => node.name === "lax");
    const sha = nodes.find(node => node.name === "sha");
    if (!nyc || !lax || !sha) throw new Error("Expected real enrolled fixture nodes");
    await run(executable("meshway-agent"), ["sync", "--state", join(temporary, "nyc.json")]);
    const initial = await getConnections(controllerUrl, admin);
    const nycIdentity = initial.nodes.find(entry => entry.node.id === nyc.id);
    expect(nycIdentity?.node.acknowledged_revision).toBe(initial.revision);
    expect(nycIdentity?.acknowledged_revision).toBe(0);
    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: controllerUrl, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    await page.goto(`${webUrl}/connections`);
    await expect(page).toHaveURL(/\/login(?:\?|$)/);
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).not.toHaveURL(/\/login(?:\?|$)/);
    await page.goto(`${webUrl}/connections`);
    await expect(page.getByTestId("connections-editor")).toBeVisible();
    await expect(page.locator('[data-slot="draft-action-bar"]')).toBeHidden();
    await expect(page.getByTestId("connection-identity-row").filter({ hasText: "pending-key" })).toContainText("未发布公钥");

    await test.step("unapplied listener edits survive browser history without moving their base", async () => {
      await page.goto(`${webUrl}/nodes`);
      await page.getByRole("link", { name: "高级连接", exact: true }).click();
      await expect(page).toHaveURL(`${webUrl}/connections`);
      const base = await page.getByTestId("connections-base-revision").innerText();
      await page.getByRole("button", { name: "接收其他节点连接", exact: true }).first().click();
      await revealAdvanced(page.getByRole("dialog"), page.getByRole("dialog").getByLabel("监听器 ID", { exact: true }));
      await page.getByRole("dialog").getByLabel("监听器 ID", { exact: true }).fill("unapplied-history-edit");
      await page.goBack();
      await expect(page).toHaveURL(`${webUrl}/nodes`);
      await page.goForward();
      await revealAdvanced(page.getByRole("dialog"), page.getByRole("dialog").getByLabel("监听器 ID", { exact: true }));
      await expect(page.getByRole("dialog").getByLabel("监听器 ID", { exact: true })).toHaveValue("unapplied-history-edit");
      await expect(page.getByTestId("connections-base-revision")).toHaveText(base);
      await page.keyboard.press("Escape");
      await page.getByRole("alertdialog").getByRole("button", { name: "放弃修改", exact: true }).click();
      await expect(page.getByRole("dialog")).not.toBeVisible();
      await expect(page.getByTestId("listener-row")).toHaveCount(0);
    });

    await test.step("create native and VLESS listeners, with SHA as dialer and LAX as acceptor", async () => {
      for (const transport of ["Native UDP", "VLESS"]) {
        const id = transport === "VLESS" ? "lax-vless" : "lax-udp";
        const port = transport === "VLESS" ? "41001" : "41000";
        await page.getByRole("button", { name: "接收其他节点连接", exact: true }).first().click();
        const sheet = page.getByRole("dialog");
        await revealAdvanced(sheet, sheet.getByLabel("监听器 ID", { exact: true }));
        await sheet.getByLabel("监听器 ID", { exact: true }).fill(id);
        await sheet.getByRole("button", { name: transport, exact: true }).click();
        await choose(page, sheet.getByLabel("监听节点", { exact: true }), /^lax/);
        await sheet.getByLabel("监听端口", { exact: true }).fill(port);
        await sheet.getByLabel("对外地址 1 主机", { exact: true }).fill("127.0.0.1");
        await sheet.getByLabel("对外地址 1 端口", { exact: true }).fill(port);
        if (transport === "VLESS") await sheet.getByLabel("监听方配置档引用", { exact: true }).fill("lax-local-profile");
        await page.keyboard.press("ControlOrMeta+k");
        await expect(page.getByRole("dialog", { name: "搜索与跳转", exact: true })).not.toBeVisible();
        await sheet.getByRole("button", { name: "添加到草稿", exact: true }).click();
        await expect(sheet).not.toBeVisible();
        await page.getByRole("button", { name: `为监听器 ${id} 添加拨号候选`, exact: true }).click();
        const candidate = page.getByRole("dialog");
        await revealAdvanced(candidate, candidate.getByLabel("候选 ID", { exact: true }));
        await candidate.getByLabel("候选 ID", { exact: true }).fill(transport === "VLESS" ? "sha-to-lax" : "nyc-to-lax");
        await choose(page, candidate.getByLabel("发起节点", { exact: true }), transport === "VLESS" ? /^sha/ : /^nyc/);
        await candidate.getByLabel("优先级", { exact: true }).fill("0");
        if (transport === "VLESS") await candidate.getByLabel("发起方配置档引用", { exact: true }).fill("sha-local-profile");
        await candidate.getByRole("button", { name: "添加到草稿", exact: true }).click();
        await expect(candidate).not.toBeVisible();
      }
      expect((await getConnections(controllerUrl, admin)).configuration.listeners).toHaveLength(0);
      const mutation = page.waitForRequest(request => request.method() === "POST" && new URL(request.url()).pathname === "/connections");
      await page.getByRole("button", { name: "保存连接配置", exact: true }).click();
      const request = await mutation;
      await expect(page.getByTestId("connections-save-status")).toContainText("已保存");
      await expect(page.getByRole("button", { name: "保存连接配置", exact: true })).toBeDisabled();
      const saved = await getConnections(controllerUrl, admin);
      const candidate = saved.configuration.candidates.find(item => item.id === "sha-to-lax");
      expect(candidate?.dialer_node_id).toBe(sha.id);
      expect(candidate?.priority).toBe(0);
      expect(saved.configuration.listeners.find(item => item.id === candidate?.listener_id)?.node_id).toBe(lax.id);
      const headers = Object.fromEntries(Object.entries(await request.allHeaders()).filter(([name]) => ["content-type", "next-action", "next-router-state-tree"].includes(name.toLowerCase())));
      const body = request.postData();
      if (!body) throw new Error("Missing connection mutation body");
      const response = await anonymous.request.post(request.url(), { headers: { ...headers, origin: webUrl }, data: body, maxRedirects: 0 });
      expect(response.headers()["x-action-redirect"] ?? response.headers().location ?? "").toContain("/login");
    });

    await test.step("endpoint rows keep unique identities through add, reopen and removal", async () => {
      await page.getByRole("button", { name: "编辑监听器 lax-udp", exact: true }).click();
      let sheet = page.getByRole("dialog");
      await sheet.getByRole("button", { name: "添加对外地址", exact: true }).click();
      await sheet.getByLabel("对外地址 2 主机", { exact: true }).fill("127.0.0.2");
      await sheet.getByLabel("对外地址 2 端口", { exact: true }).fill("41000");
      await sheet.getByRole("button", { name: "应用到草稿", exact: true }).click();
      await expect(sheet).not.toBeVisible();
      await page.getByRole("button", { name: "编辑监听器 lax-udp", exact: true }).click();
      sheet = page.getByRole("dialog");
      await sheet.getByRole("button", { name: "添加对外地址", exact: true }).click();
      await sheet.getByLabel("对外地址 3 主机", { exact: true }).fill("127.0.0.3");
      await sheet.getByLabel("对外地址 3 端口", { exact: true }).fill("41000");
      expect(await sheet.locator("input[id]").evaluateAll(elements => new Set(elements.map(element => element.id)).size === elements.length)).toBe(true);
      await sheet.getByRole("button", { name: "移除对外地址 1", exact: true }).click();
      await expect(sheet.getByLabel(/^对外地址 \d+ 主机$/)).toHaveCount(2);
      await expect(sheet.getByLabel("对外地址 1 主机", { exact: true })).toHaveValue("127.0.0.2");
      await expect(sheet.getByLabel("对外地址 2 主机", { exact: true })).toHaveValue("127.0.0.3");
      await sheet.getByRole("button", { name: "添加对外地址", exact: true }).click();
      await sheet.getByLabel("对外地址 3 主机", { exact: true }).fill("127.0.0.1");
      await sheet.getByLabel("对外地址 3 端口", { exact: true }).fill("41000");
      expect(await sheet.locator("input[id]").evaluateAll(elements => new Set(elements.map(element => element.id)).size === elements.length)).toBe(true);
      await sheet.getByRole("button", { name: "应用到草稿", exact: true }).click();
      await page.getByRole("button", { name: "保存连接配置", exact: true }).click();
      await expect.poll(async () => (await getConnections(controllerUrl, admin)).configuration.listeners.find(listener => listener.id === "lax-udp")?.advertised_endpoints.length).toBe(3);
      await expect(page.getByRole("button", { name: "保存连接配置", exact: true })).toBeDisabled();
      const saved = await getConnections(controllerUrl, admin);
      expect(saved.configuration.listeners.find(listener => listener.id === "lax-udp")?.advertised_endpoints.map(endpoint => endpoint.host)).toEqual(["127.0.0.1", "127.0.0.2", "127.0.0.3"]);
    });

    await test.step("connection receipts are persisted separately and do not claim applied networking", async () => {
      for (const node of ["nyc", "lax", "sha"]) {
        const state = join(temporary, `${node}.json`);
        const response = await run(executable("meshway-agent"), ["connections", "sync", "--state", state]);
        expect(response.stdout).toContain('"applied": false');
        expect((await stat(`${state}.connections.json`)).mode & 0o777).toBe(0o600);
      }
      const saved = await getConnections(controllerUrl, admin);
      expect(saved.nodes.filter(entry => entry.node.name !== "pending-key").every(entry => entry.acknowledged_revision === saved.revision)).toBe(true);
      await page.reload();
      await expect(page.locator('[data-slot="draft-action-bar"]')).toBeHidden();
      const content = await page.content();
      for (const value of secrets) expect(content.includes(value), "credentials must not appear in HTML/Flight").toBe(false);
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.evaluate(() => localStorage.setItem("theme", "light"));
      await page.reload();
      await page.evaluate(() => window.scrollTo(0, 0));
      await page.screenshot({ path: test.info().outputPath("connections-desktop-light.png"), fullPage: true });
      await page.evaluate(() => localStorage.setItem("theme", "dark"));
      await page.reload();
      await page.evaluate(() => window.scrollTo(0, 0));
      await page.screenshot({ path: test.info().outputPath("connections-desktop-dark.png"), fullPage: true });
    });

    await test.step("node-scoped saves survive tab changes and fresh same-revision receipts arrive automatically", async () => {
      await page.goto(`${webUrl}/nodes/${lax.id}?tab=connections`);
      await expandDetails(page, "高级手工连接配置");
      await expect(page.getByRole("heading", { name: "lax", exact: true })).toBeVisible();
      await expect(page.getByTestId("connections-editor")).toHaveAttribute("data-scope", lax.id);
      await page.getByRole("button", { name: "编辑监听器 lax-udp", exact: true }).click();
      const sheet = page.getByRole("dialog");
      await revealAdvanced(sheet, sheet.getByLabel("监听器 ID", { exact: true }));
      await sheet.getByLabel("监听器 ID", { exact: true }).fill("lax-udp-renamed");
      await sheet.getByRole("button", { name: "应用到草稿", exact: true }).click();
      await page.getByRole("button", { name: "保存连接配置", exact: true }).click();
      await expect.poll(async () => (await getConnections(controllerUrl, admin)).configuration.listeners.some(listener => listener.id === "lax-udp-renamed")).toBe(true);
      const saved = await getConnections(controllerUrl, admin);
      expect(saved.configuration.candidates.find(candidate => candidate.id === "nyc-to-lax")?.listener_id).toBe("lax-udp-renamed");
      expect(saved.configuration.candidates.find(candidate => candidate.id === "sha-to-lax")?.listener_id).toBe("lax-vless");
      await page.getByRole("tab", { name: "概览", exact: true }).click();
      await page.getByRole("tab", { name: "连接", exact: true }).click();
      await expandDetails(page, "高级手工连接配置");
      await expect(page.getByTestId("connections-base-revision")).toHaveText(`r${saved.revision}`);
      await expect(page.getByTestId("listener-row").filter({ hasText: "lax-udp-renamed" })).toBeVisible();
      await run(executable("meshway-agent"), ["connections", "sync", "--state", join(temporary, "lax.json")]);
      await expect(page.getByText("已保存当前版本", { exact: true }).first()).toBeVisible({ timeout: 20_000 });
      await expect(page.getByTestId("connection-runtime")).toHaveAttribute("data-freshness", "unreported");
      await page.goto(`${webUrl}/connections`);
    });

    await test.step("navigation retains a dirty draft and a competing global writer causes CAS conflict", async () => {
      const base = await page.getByTestId("connections-base-revision").innerText();
      await page.getByRole("button", { name: "编辑拨号候选 sha-to-lax", exact: true }).click();
      await revealAdvanced(page.getByRole("dialog"), page.getByRole("dialog").getByLabel("优先级", { exact: true }));
      await page.getByRole("dialog").getByLabel("优先级", { exact: true }).fill("20");
      await page.getByRole("dialog").getByRole("button", { name: "应用到草稿", exact: true }).click();
      await page.keyboard.press("ControlOrMeta+k");
      await page.getByRole("option", { name: "节点", exact: true }).click();
      await expect(page).toHaveURL(`${webUrl}/nodes`);
      const latest = await getConnections(controllerUrl, admin);
      await setNodeCountry(controllerUrl, admin, { node_id: nyc.id, country: "US", expected_revision: latest.revision });
      await page.goBack();
      await expect(page.getByTestId("connections-base-revision")).toHaveText(base);
      await page.getByRole("button", { name: "保存连接配置", exact: true }).click();
      await expect(page.getByTestId("connections-save-status")).toContainText("冲突");
      await expect(page.getByTestId("connection-candidate-row").filter({ hasText: "sha-to-lax" })).toContainText("20");
      expect((await getConnections(controllerUrl, admin)).configuration.candidates.find(item => item.id === "sha-to-lax")?.priority).toBe(0);
      await page.getByRole("button", { name: "放弃修改", exact: true }).click();
      await page.getByRole("alertdialog").getByRole("button", { name: "放弃修改", exact: true }).click();
    });

    await test.step("revocation keeps source references for repair and the mobile view remains usable", async () => {
      await revokeNode(controllerUrl, admin, { node_id: lax.id });
      await page.reload();
      await expect(page.getByTestId("connections-editor")).toContainText("已保存的连接配置当前无效");
      await expect(page.getByTestId("listener-row")).toHaveCount(2);
      await expect(page.getByTestId("connection-candidate-row")).toHaveCount(2);
      await run(executable("meshway-agent"), ["connections", "sync", "--state", join(temporary, "nyc.json")]);
      await page.setViewportSize({ width: 390, height: 844 });
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await page.evaluate(() => window.scrollTo(0, 0));
      await page.screenshot({ path: test.info().outputPath("connections-mobile.png"), fullPage: true });
      const snapshot = await getConnections(controllerUrl, admin);
      await saveConnections(controllerUrl, admin, { expected_revision: snapshot.revision, configuration: { listeners: [], candidates: [] } });
    });
    expect(browserErrors).toEqual([]);
  } finally {
    await anonymous.close();
    await web?.stop();
    await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
