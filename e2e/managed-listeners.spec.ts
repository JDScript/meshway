import { expect, test, type Page } from "@playwright/test";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer, request as forward, type Server } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  createEnrollmentToken,
  createNetwork,
  getConnections,
  getManagedListeners,
  listNetworks,
  listNodes,
  saveConnections,
  saveManagedListener,
} from "../packages/control-client/src/index.ts";
import {
  getAgentManagedConnectionsOperation,
  publishManagedTransportCapabilitiesOperation,
} from "../packages/control-client/src/generated/operations.gen.ts";
import { executable, expectNoOverflow, root, run, service, unusedPort } from "./harness.ts";
import { revealAdvanced } from "./web-actions.ts";

const SHOTS = process.env.MESHWAY_SHOTS_DIR;

async function nodeCredential(path: string): Promise<string> {
  const value: unknown = JSON.parse(await readFile(path, "utf8"));
  if (typeof value === "object" && value !== null && "node_credential" in value && typeof value.node_credential === "string") return value.node_credential;
  throw new Error("Private fixture field is missing");
}

/** Full page, or only the viewport while a modal sheet is open. */
async function shoot(page: Page, name: string, fullPage = true) {
  // Let sheet / theme transitions finish so the capture shows the settled layout.
  await page.evaluate(() => Promise.all(document.getAnimations().map((animation) => animation.finished.catch(() => undefined))));
  await page.screenshot({ path: test.info().outputPath(`${name}.png`), fullPage });
  if (SHOTS) {
    await mkdir(SHOTS, { recursive: true });
    await page.screenshot({ path: join(SHOTS, `${name}.png`), fullPage });
  }
}

/**
 * Loopback proxy between the web server and the controller. While `failManaged`
 * is set, only the managed listener read answers 500, so every other source
 * of the same poll keeps succeeding.
 */
async function controllerProxy(target: string, port: number) {
  const state = { failManaged: false, managedReads: 0 };
  const upstream = new URL(target);
  const server: Server = createServer((incoming, outgoing) => {
    const path = incoming.url ?? "/";
    if (incoming.method === "GET" && /\/managed-listeners$/.test(path)) {
      state.managedReads += 1;
      if (state.failManaged) {
        incoming.resume();
        outgoing.writeHead(500, { "content-type": "application/json" }).end(JSON.stringify({ message: "injected managed read failure" }));
        return;
      }
    }
    const proxied = forward({ host: upstream.hostname, port: upstream.port, method: incoming.method, path, headers: incoming.headers }, (response) => {
      outgoing.writeHead(response.statusCode ?? 502, response.headers);
      response.pipe(outgoing);
    });
    proxied.on("error", () => outgoing.writeHead(502).end());
    incoming.pipe(proxied);
  });
  await new Promise<void>((resolve) => server.listen(port, "127.0.0.1", resolve));
  return { state, url: `http://127.0.0.1:${port}`, close: () => new Promise<void>((resolve) => server.close(() => resolve())) };
}

async function theme(page: Page, value: "light" | "dark") {
  await page.evaluate((next) => localStorage.setItem("theme", next), value);
}

test("managed listeners: optional listener at enrollment, node configuration edits, CAS, isolation and secrecy", async ({ page, browser }) => {
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-managed-listeners-e2e-"));
  const controllerPort = await unusedPort();
  const webPort = await unusedPort();
  const base = `http://127.0.0.1:${controllerPort}`;
  const webUrl = `http://127.0.0.1:${webPort}`;
  const controller = service(executable("meshway-controller"), ["serve", "--data-dir", join(temporary, "controller"), "--bind", `127.0.0.1:${controllerPort}`]);
  let web: ReturnType<typeof service> | undefined;
  let proxy: Awaited<ReturnType<typeof controllerProxy>> | undefined;
  const anonymous = await browser.newContext();
  const browserErrors: string[] = [];
  page.on("pageerror", (error) => browserErrors.push(error.message));
  // Unsaved forms warn on unload; full-page navigations in this test accept.
  page.on("dialog", (dialog) => void dialog.accept());
  try {
    await controller.ready(`${base}/api/v1/networks`, 401);
    const admin = (await readFile(join(temporary, "controller/admin.token"), "utf8")).trim();
    const first = (await listNetworks(base, admin)).networks[0]!;
    const second = await createNetwork(base, admin, { name: "Isolated network" });
    const secrets = [admin];
    proxy = await controllerProxy(base, await unusedPort());
    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: proxy.url, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    const at = (networkId: string, path = "") => `${webUrl}/${networkId}${path}`;
    await page.goto(at(first.network_id, "/machines/add"));
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).not.toHaveURL(/\/login(?:\?|$)/);

    // A dialer node enrolled with a plain token (no listener).
    const dialerToken = await createEnrollmentToken(base, admin, first.network_id, { expires_in_seconds: 600 });
    const dialerState = join(temporary, "dialer.json");
    await run(executable("meshway-agent"), ["enroll", "--controller", base, "--token", dialerToken.token, "--name", "dialer", "--state", dialerState]);
    await run(executable("meshway-agent"), ["connections", "publish", "--state", dialerState]);
    const dialerCredential = await nodeCredential(dialerState);
    // Generic VLESS capability does not imply support for its managed stream
    // variants. Both sides of the later WS candidate must declare those facts.
    await publishManagedTransportCapabilitiesOperation(base, dialerCredential, { vless_streams: ["tcp", "websocket"] });
    secrets.push(dialerCredential);

    let edgeState = "";
    let edgeId = "";
    await test.step("add node: an optional WebSocket listener rides on the one-time token", async () => {
      await page.goto(at(first.network_id, "/machines/add"));
      await expect(page.getByTestId("enrollment-listener")).toHaveCount(0);
      await page.getByRole("switch", { name: "同时配置监听", exact: true }).click();
      const panel = page.getByTestId("enrollment-listener");
      await expect(panel).toBeVisible();
      await panel.getByRole("button", { name: "VLESS · WebSocket", exact: true }).click();
      await panel.getByLabel("监听端口", { exact: true }).fill("18443");
      await panel.getByRole("button", { name: "添加对外地址", exact: true }).click();
      await panel.getByLabel("对外地址 1 主机", { exact: true }).fill("edge.example.test");
      await panel.getByLabel("WebSocket 路径", { exact: true }).fill("/bad path");
      await revealAdvanced(panel, panel.getByLabel("监听 ID", { exact: true }));
      await panel.getByLabel("监听 ID", { exact: true }).fill("edge-ws");
      await page.getByRole("button", { name: "生成令牌并绑定监听", exact: true }).click();
      await expect(panel.getByText("路径不能包含空格或换行", { exact: true })).toBeVisible();
      await expect(page.getByTestId("enrollment-token")).toHaveCount(0);

      // The unsaved listener belongs to this network: the other network's add page starts clean,
      // and switching back (in-app) restores it.
      const switchTo = async (network: { network_id: string; name: string }) => {
        await page.getByRole("button", { name: "当前网络", exact: true }).click();
        await page.getByRole("menuitem", { name: network.name, exact: true }).click();
        await expect(page).toHaveURL(at(network.network_id, "/machines"));
        await page.keyboard.press("ControlOrMeta+k");
        await page.getByRole("option", { name: /添加节点/ }).click();
        await expect(page).toHaveURL(at(network.network_id, "/machines/add"));
      };
      await switchTo(second);
      await expect(page.getByRole("switch", { name: "同时配置监听", exact: true })).toHaveAttribute("aria-checked", "false");
      await switchTo(first);
      await expect(page.getByLabel("WebSocket 路径", { exact: true })).toHaveValue("/bad path");

      await page.getByLabel("WebSocket 路径", { exact: true }).fill("/meshway");
      await page.getByRole("button", { name: "生成令牌并绑定监听", exact: true }).click();
      const summary = page.getByTestId("enrollment-listener-summary");
      await expect(summary).toContainText("edge-ws");
      await expect(summary).toContainText("其他节点仍需拨号候选");
      const token = (await page.getByTestId("enrollment-token").innerText()).trim();
      // Only the token exists; no listener is stored until a node consumes it.
      expect((await getManagedListeners(base, admin, first.network_id)).listeners).toEqual([]);
      edgeState = join(temporary, "edge.json");
      await run(executable("meshway-agent"), ["enroll", "--controller", base, "--token", token, "--name", "edge", "--state", edgeState]);
      secrets.push(token, await nodeCredential(edgeState));
      const nodes = (await listNodes(base, admin, first.network_id)).nodes;
      edgeId = nodes.find((node) => node.name === "edge")!.id;
      const saved = (await getManagedListeners(base, admin, first.network_id)).listeners;
      expect(saved.map((item) => item.settings)).toEqual([{
        id: "edge-ws", node_id: edgeId, enabled: true, transport: "vless", bind_address: "0.0.0.0", bind_port: 18443,
        advertised_endpoints: [{ host: "edge.example.test", port: 18443 }],
        vless: { stream: "websocket", server_name: "meshway.local", websocket_path: "/meshway", websocket_host: "edge.example.test" },
      }]);
      expect(saved[0]?.compiled_capability_available).toBe(false);
      await shoot(page, "add-node-token-desktop-light");
    });

    const nodePage = at(first.network_id, `/machines/${edgeId}?tab=connections`);
    await test.step("node configuration: managed facts, capability state and no secrets", async () => {
      await page.goto(nodePage);
      const row = page.getByTestId("managed-listener").filter({ hasText: "VLESS · WebSocket" });
      await expect(row).toContainText("当前承载能力不可用");
      await expect(row).toContainText("/meshway");
      await expect(row).toContainText("材料 v1");
      // Capability reports come from the node itself (its real credential).
      const credential = await nodeCredential(edgeState);
      await run(executable("meshway-agent"), ["connections", "publish", "--state", edgeState]);
      await publishManagedTransportCapabilitiesOperation(base, credential, { vless_streams: ["tcp", "websocket"] });
      await page.reload();
      await expect(page.getByTestId("managed-listener")).toContainText("承载能力可用");
      // Node-only material (UUID, PEM, private key) never reaches the browser.
      const material = (await getAgentManagedConnectionsOperation(base, credential)).materials[0]!;
      expect(material.private_key_pem).toBeTruthy();
      secrets.push(material.uuid, material.private_key_pem!, material.certificate_pem.split("\n")[1]!);
      const content = await page.content();
      for (const value of secrets) expect(content.includes(value), "secrets must not appear in HTML / Flight").toBe(false);
      await expect(page.getByTestId("carrier-listener").filter({ hasText: "edge-ws" })).toContainText("VLESS 不做 UDP 地址观测");
    });

    await test.step("edit: field errors keep input, the draft survives tab switches, save rotates nothing but the version", async () => {
      await page.getByRole("button", { name: "编辑监听 edge-ws", exact: true }).click();
      const sheet = page.getByRole("dialog");
      await expect(sheet.getByRole("button", { name: "Native UDP", exact: true })).toBeDisabled();
      await revealAdvanced(sheet, sheet.getByLabel("监听 ID", { exact: true }));
      await expect(sheet.getByLabel("监听 ID", { exact: true })).toHaveAttribute("readonly", "");
      await sheet.getByLabel("监听地址", { exact: true }).fill("edge.example.test");
      await sheet.getByRole("button", { name: "保存监听", exact: true }).click();
      await expect(sheet.getByText(/必须是本机 IP 地址/)).toBeVisible();
      await sheet.getByLabel("监听地址", { exact: true }).fill("127.0.0.1");
      await sheet.getByLabel("WebSocket 路径", { exact: true }).fill("/mesh2");
      // Close for later, switch tabs (background reads keep arriving), come back: the form is intact.
      await page.keyboard.press("Escape");
      await page.getByRole("alertdialog").getByRole("button", { name: "稍后继续", exact: true }).click();
      await expect(sheet).toBeHidden();
      await expect(page.getByTestId("managed-listener")).toContainText("有未保存的修改");
      await page.getByRole("tab", { name: "概览", exact: true }).click();
      await page.getByRole("tab", { name: "配置", exact: true }).click();
      await page.getByRole("button", { name: "编辑监听 edge-ws", exact: true }).click();
      await expect(page.getByRole("dialog").getByLabel("WebSocket 路径", { exact: true })).toHaveValue("/mesh2");
      await expect(page.getByRole("dialog").getByLabel("监听地址", { exact: true })).toHaveValue("127.0.0.1");
      await shoot(page, "node-listener-sheet-desktop-light", false);
      const mutation = page.waitForRequest((request) => request.method() === "POST" && (request.postData() ?? "").includes("/mesh2"));
      await page.getByRole("dialog").getByRole("button", { name: "保存监听", exact: true }).click();
      const request = await mutation;
      await expect(page.getByRole("dialog")).toBeHidden();
      const saved = (await getManagedListeners(base, admin, first.network_id)).listeners[0]!;
      expect(saved.settings.vless?.websocket_path).toBe("/mesh2");
      expect(saved.settings.bind_address).toBe("127.0.0.1");
      expect(saved.material_version).toBe(2);
      await expect(page.getByTestId("managed-listener")).toContainText("材料 v2");

      // An anonymous replay of the same Server Action is sent to login and changes nothing.
      const headers = Object.fromEntries(Object.entries(await request.allHeaders()).filter(([name]) => ["content-type", "next-action", "next-router-state-tree"].includes(name.toLowerCase())));
      const body = request.postData();
      if (!body) throw new Error("Missing managed listener action body");
      const before = (await getManagedListeners(base, admin, first.network_id)).revision;
      const replay = await anonymous.request.post(request.url(), { headers: { ...headers, origin: webUrl }, data: body, maxRedirects: 0 });
      expect(replay.headers()["x-action-redirect"] ?? replay.headers().location ?? "").toContain("/login");
      expect((await getManagedListeners(base, admin, first.network_id)).revision).toBe(before);
    });

    await test.step("CAS: conflict keeps the form; after an explicit rebase the adopted version decides what is a change", async () => {
      await page.getByRole("button", { name: "编辑监听 edge-ws", exact: true }).click();
      const sheet = page.getByRole("dialog");
      const port = sheet.getByLabel("监听端口", { exact: true });
      const saveButton = sheet.getByRole("button", { name: "保存监听", exact: true });
      await expect(port).toHaveValue("18443");
      await port.fill("8443");
      // A competing writer saves this listener with 9443.
      const competing = await getManagedListeners(base, admin, first.network_id);
      const current = competing.listeners.find((item) => item.settings.id === "edge-ws")!.settings;
      await saveManagedListener(base, admin, first.network_id, { expected_revision: competing.revision, settings: { ...current, bind_port: 9443 } });
      await saveButton.click();
      await expect(sheet.getByTestId("managed-listener-outcome")).toHaveAttribute("data-status", "conflict");
      await expect(port).toHaveValue("8443");
      await sheet.getByRole("button", { name: "读取最新版本", exact: true }).click();
      await expect(sheet.getByTestId("managed-listener-rebased")).toContainText("本机监听");
      await expect(port).toHaveValue("8443");
      // The newly adopted saved value is "no change"; the value first opened is a real change.
      await port.fill("9443");
      await expect(saveButton).toBeDisabled();
      await expect(sheet.getByText("没有修改", { exact: true })).toBeVisible();
      await port.fill("18443");
      await expect(saveButton).toBeEnabled();
      await expect(sheet.getByText("未保存的修改", { exact: true })).toBeVisible();
      const saves: string[] = [];
      const collect = (request: { method(): string; postData(): string | null }) => {
        const body = request.method() === "POST" ? request.postData() ?? "" : "";
        if (body.includes('"bind_port":18443')) saves.push(body);
      };
      page.on("request", collect);
      await saveButton.click();
      await expect(sheet).toBeHidden();
      page.off("request", collect);
      expect(saves).toHaveLength(1);
      const saved = await getManagedListeners(base, admin, first.network_id);
      expect(saved.listeners.find((item) => item.settings.id === "edge-ws")?.settings.bind_port).toBe(18443);
      expect(saved.revision).toBe(competing.revision + 2);
    });

    await test.step("a failing managed read alone shows a dated cache with retry; listeners stay managed", async () => {
      proxy!.state.failManaged = true;
      const failure = page.getByTestId("managed-read-failure");
      // The next background poll reads everything; only the managed part fails.
      await expect(failure).toHaveAttribute("data-freshness", "cached", { timeout: 30_000 });
      await expect(failure).toContainText("的缓存（r");
      await expect(page.getByTestId("node-status-stale")).toHaveCount(0);
      await expect(page.getByTestId("managed-listener").filter({ hasText: "VLESS · WebSocket" })).toContainText("缓存 · 托管设置 r");
      await expect(page.getByTestId("manual-listener")).toHaveCount(0);
      // Editing from the cache is possible and says so.
      await page.getByRole("button", { name: "编辑监听 edge-ws", exact: true }).click();
      await expect(page.getByRole("dialog").getByTestId("managed-listener-cached")).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(page.getByRole("dialog")).toBeHidden();
      // A retry while the API still fails keeps the error visible.
      const reads = proxy!.state.managedReads;
      await failure.getByRole("button", { name: "重试", exact: true }).click();
      await expect.poll(() => proxy!.state.managedReads).toBeGreaterThan(reads);
      await expect(failure).toHaveAttribute("data-freshness", "cached");
      proxy!.state.failManaged = false;
      await failure.getByRole("button", { name: "重试", exact: true }).click();
      await expect(page.getByTestId("managed-read-failure")).toHaveCount(0);
      await expect(page.getByTestId("managed-listener").filter({ hasText: "VLESS · WebSocket" })).not.toContainText("缓存");
    });

    await test.step("disable: referencing candidates are shown first and kept; the advanced editor is read-only for it", async () => {
      const dialer = (await listNodes(base, admin, first.network_id)).nodes.find((node) => node.name === "dialer")!;
      const current = await getConnections(base, admin, first.network_id);
      await saveConnections(base, admin, first.network_id, {
        expected_revision: current.revision,
        configuration: {
          listeners: current.configuration.listeners,
          candidates: [{ id: "dialer-to-edge", dialer_node_id: dialer.id, listener_id: "edge-ws", endpoint: { host: "edge.example.test", port: 18443 }, priority: 0, credential_ref: "managed.edge-ws" }],
        },
      });
      await page.reload();
      await page.getByRole("button", { name: "编辑监听 edge-ws", exact: true }).click();
      const sheet = page.getByRole("dialog");
      await expect(sheet.getByTestId("managed-listener-impact")).toContainText("dialer");
      await sheet.getByRole("switch", { name: "启用监听", exact: true }).click();
      await expect(sheet.getByTestId("managed-listener-impact")).toContainText("停用后这 1 个拨号候选仍保留");
      await sheet.getByRole("button", { name: "保存监听", exact: true }).click();
      await expect(sheet).toBeHidden();
      const disabled = page.getByTestId("carrier-listener-disabled");
      await expect(disabled).toContainText("已停用");
      await expect(disabled).toContainText("dialer-to-edge");
      const after = await getConnections(base, admin, first.network_id);
      expect(after.configuration.candidates.map((candidate) => candidate.id)).toEqual(["dialer-to-edge"]);
      expect(after.valid).toBe(false);
      // Re-enable through the same form restores the reference.
      await page.getByRole("button", { name: "编辑监听 edge-ws", exact: true }).click();
      await page.getByRole("dialog").getByRole("switch", { name: "启用监听", exact: true }).click();
      await page.getByRole("dialog").getByRole("button", { name: "保存监听", exact: true }).click();
      await expect(page.getByRole("dialog")).toBeHidden();
      await expect.poll(async () => (await getConnections(base, admin, first.network_id)).valid).toBe(true);
      // The advanced editor links managed listeners to node configuration and offers no delete.
      await page.goto(at(first.network_id, "/connections"));
      const legacyRow = page.getByTestId("listener-row").filter({ hasText: "edge-ws" });
      await expect(legacyRow).toContainText("平台托管");
      await expect(legacyRow.getByRole("button", { name: "删除监听器 edge-ws", exact: true })).toHaveCount(0);
      await legacyRow.getByRole("link", { name: "在节点配置中编辑托管监听 edge-ws", exact: true }).click();
      await expect(page).toHaveURL(new RegExp(`/machines/${edgeId}\\?tab=connections`));
    });

    await test.step("create on the node page: native TCP with explicit endpoint; network isolation", async () => {
      await page.getByTestId("managed-listener-create").click();
      const sheet = page.getByRole("dialog");
      await sheet.getByRole("button", { name: "Native TCP", exact: true }).click();
      await expect(sheet.getByLabel("监听端口", { exact: true })).toHaveValue("8421");
      await revealAdvanced(sheet, sheet.getByLabel("监听 ID", { exact: true }));
      await sheet.getByLabel("监听 ID", { exact: true }).fill("edge-ws");
      await sheet.getByRole("button", { name: "创建监听", exact: true }).click();
      await expect(sheet.getByText("本网络已有同名监听；ID 在整张网络内唯一", { exact: true })).toBeVisible();
      await sheet.getByLabel("监听 ID", { exact: true }).fill("edge-tcp");
      await sheet.getByRole("button", { name: "创建监听", exact: true }).click();
      await expect(sheet).toBeHidden();
      const listeners = (await getManagedListeners(base, admin, first.network_id)).listeners.map((item) => item.settings);
      expect(listeners.find((item) => item.id === "edge-tcp")).toMatchObject({ transport: "native_tcp", bind_port: 8421, advertised_endpoints: [], vless: null });
      expect((await getManagedListeners(base, admin, second.network_id)).listeners).toEqual([]);
      expect((await page.goto(at(second.network_id, `/machines/${edgeId}?tab=connections`)))?.status()).toBe(404);
      // Bypassing the UI does not bypass the controller: a WebSocket field on TCP is a 422.
      const snapshot = await getManagedListeners(base, admin, first.network_id);
      await expect(saveManagedListener(base, admin, first.network_id, {
        expected_revision: snapshot.revision,
        settings: { ...listeners.find((item) => item.id === "edge-ws")!, vless: { stream: "tcp", server_name: "meshway.local", websocket_path: "/x", websocket_host: null } },
      })).rejects.toMatchObject({ status: 422 });
    });

    await test.step("screenshots: 1440 / 390, light / dark", async () => {
      for (const mode of ["light", "dark"] as const) {
        await page.setViewportSize({ width: 1440, height: 1000 });
        await page.goto(nodePage);
        await theme(page, mode);
        await page.reload();
        await expect(page.getByTestId("managed-listener").first()).toBeVisible();
        await shoot(page, `node-configuration-desktop-${mode}`);
        await page.setViewportSize({ width: 390, height: 844 });
        await page.reload();
        await expectNoOverflow(page);
        await shoot(page, `node-configuration-mobile-${mode}`);
        await page.getByRole("button", { name: "编辑监听 edge-ws", exact: true }).click();
        await expect(page.getByRole("dialog")).toBeVisible();
        await shoot(page, `node-listener-sheet-mobile-${mode}`, false);
        await page.keyboard.press("Escape");
        await page.goto(at(first.network_id, "/machines/add"));
        await page.getByRole("switch", { name: "同时配置监听", exact: true }).click();
        await page.getByTestId("enrollment-listener").getByRole("button", { name: "VLESS · WebSocket", exact: true }).click();
        await expectNoOverflow(page);
        await shoot(page, `add-node-listener-mobile-${mode}`);
        await page.setViewportSize({ width: 1440, height: 1000 });
        await shoot(page, `add-node-listener-desktop-${mode}`);
      }
    });

    await test.step("logout drops unsaved listener forms (no full reload in between)", async () => {
      await expect(page.getByRole("switch", { name: "同时配置监听", exact: true })).toHaveAttribute("aria-checked", "true");
      await page.getByRole("button", { name: "账户菜单", exact: true }).click();
      await page.getByRole("menuitem", { name: "退出登录", exact: true }).click();
      await expect(page).toHaveURL(/\/login(?:\?|$)/);
      await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
      await page.getByRole("button", { name: "登录", exact: true }).click();
      await expect(page).not.toHaveURL(/\/login(?:\?|$)/);
      await page.getByRole("button", { name: "管理网络", exact: true }).click();
      await page.keyboard.press("ControlOrMeta+k");
      await page.getByRole("option", { name: /添加节点/ }).click();
      await expect(page).toHaveURL(at(first.network_id, "/machines/add"));
      await expect(page.getByRole("switch", { name: "同时配置监听", exact: true })).toHaveAttribute("aria-checked", "false");
    });
    expect(browserErrors).toEqual([]);
  } finally {
    await anonymous.close();
    await web?.stop();
    await proxy?.close();
    await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
