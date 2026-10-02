import { expect, test } from "@playwright/test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEnrollmentToken, getConnections, listNodes, saveConnections, setNodeCountry } from "./network-client.ts";
import { beginConnectionRuntimeOperation, getAgentConnectionsOperation, reportConnectionRuntimeOperation } from "../packages/control-client/src/generated/operations.gen.ts";
import { consoleAt, executable, HEADER_HEIGHT, overviewStage, root, run, service, SIDEBAR_WIDTH, unusedPort } from "./harness.ts";

async function credentialOf(state: string): Promise<string> {
  const parsed: unknown = JSON.parse(await readFile(state, "utf8"));
  if (typeof parsed !== "object" || parsed === null || !("node_credential" in parsed) || typeof parsed.node_credential !== "string") throw new Error("Missing fixture credential");
  return parsed.node_credential;
}

test("the overview map shows only sourced places and reported links, and opens into management", async ({ page }) => {
  test.setTimeout(180_000);
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-overview-e2e-"));
  const controllerPort = await unusedPort();
  const webPort = await unusedPort();
  const base = `http://127.0.0.1:${controllerPort}`;
  const webUrl = `http://127.0.0.1:${webPort}`;
  const startController = () => service(executable("meshway-controller"), ["serve", "--data-dir", join(temporary, "controller"), "--bind", `127.0.0.1:${controllerPort}`]);
  let controller = startController();
  let web: ReturnType<typeof service> | undefined;
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await controller.ready(`${base}/api/v1/overview`, 401);
    const admin = (await readFile(join(temporary, "controller/admin.token"), "utf8")).trim();
    const states: Record<string, string> = {};
    for (const name of ["hk-relay", "sg-edge"]) {
      const grant = await createEnrollmentToken(base, admin, { expires_in_seconds: 300 });
      states[name] = join(temporary, `${name}.json`);
      await run(executable("meshway-agent"), ["enroll", "--controller", base, "--token", grant.token, "--name", name, "--state", states[name]]);
      await run(executable("meshway-agent"), ["connections", "publish", "--state", states[name]]);
    }
    const nodes = (await listNodes(base, admin)).nodes;
    const hk = nodes.find(node => node.name === "hk-relay");
    const sg = nodes.find(node => node.name === "sg-edge");
    if (!hk || !sg) throw new Error("Missing fixture nodes");
    // Legacy administrator country: must never become a map position.
    const original = await getConnections(base, admin);
    await setNodeCountry(base, admin, { node_id: hk.id, country: "HK", expected_revision: original.revision });
    const plan = await getConnections(base, admin);
    await saveConnections(base, admin, { expected_revision: plan.revision, configuration: {
      listeners: [{ id: "sg-udp", node_id: sg.id, transport: "native_udp", bind: "127.0.0.1:45301", advertised_endpoints: [{ host: "127.0.0.1", port: 45301 }], credential_ref: null }],
      candidates: [{ id: "hk-to-sg", dialer_node_id: hk.id, listener_id: "sg-udp", endpoint: { host: "127.0.0.1", port: 45301 }, priority: 1, credential_ref: null }],
    } });

    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: base, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    const ui = await consoleAt(webUrl, base, admin);
    await page.goto(`${webUrl}/login`);
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect.poll(() => new URL(page.url()).pathname).toBe(ui.path());

    // Signed in: the immersive map, no sidebar or header.
    const overview = page.getByTestId("network-overview");
    await expect(overview).toHaveAttribute("data-immersive", "true");
    await expect.poll(() => overviewStage(page)).toMatchObject({ top: 0, bottom: 0, header: 0 });
    await expect(page.getByTestId("overview-lens")).toContainText("2 个节点待定位");
    // The off-canvas sidebar and hidden header are out of the tab order and the accessibility tree.
    await expect(page.getByRole("button", { name: "当前网络", exact: true })).toHaveCount(0);
    await expect(page.getByRole("navigation", { name: "控制台" })).toHaveCount(0);
    const tabStops: string[] = [];
    for (let index = 0; index < 12; index += 1) {
      await page.keyboard.press("Tab");
      tabStops.push(await page.evaluate(() => {
        const active = document.activeElement;
        if (!active || active === document.body) return "body";
        if (active.closest('[data-slot="sidebar"], [data-slot="console-topbar"]')) return "hidden-shell";
        return active.getAttribute("aria-label") ?? active.textContent?.trim() ?? active.tagName;
      }));
    }
    expect(tabStops).not.toContain("hidden-shell");
    expect(tabStops).toContain("管理网络");
    // Configured, but no agent has reported: drawn as configuration only.
    const edge = page.getByTestId("overview-edge");
    await expect(edge).toHaveCount(1);
    await expect(edge).toHaveAttribute("data-look", "configured");
    await expect(page.getByTestId("overview-status")).toContainText("0 / 1 连接双向确认");
    await expect(page.getByTestId("overview-status")).toContainText("1 未验证");

    // Keyboard: a node opens its summary; position stays unknown despite the country label.
    await page.getByRole("button", { name: /^hk-relay，/ }).focus();
    await page.keyboard.press("Enter");
    const card = page.getByTestId("overview-selection");
    await expect(card).toContainText("待定位（尚无位置来源）");
    await expect(card).not.toContainText("HK");
    await expect(card.getByRole("link", { name: "节点配置" })).toHaveAttribute("href", ui.path(`/machines/${hk.id}?tab=connections`));
    await page.keyboard.press("Escape");
    await expect(card).toHaveCount(0);

    // Both agents report a fresh authenticated probe: confirmed, each side's own RTT.
    const config = { hk: await credentialOf(states["hk-relay"] ?? ""), sg: await credentialOf(states["sg-edge"] ?? "") };
    for (const [credential, peer] of [
      [config.hk, { peer_node_id: sg.id, transport: "native_udp", initiator_node_id: hk.id, listener_id: "sg-udp", candidate_id: "hk-to-sg", state: "authenticated", reason: "none", probe_fresh: true, probe_rtt_ms: 31 }],
      [config.sg, { peer_node_id: hk.id, transport: "native_udp", initiator_node_id: hk.id, listener_id: "sg-udp", candidate_id: null, state: "authenticated", reason: "none", probe_fresh: true, probe_rtt_ms: 29 }],
    ] as const) {
      const snapshot = await getAgentConnectionsOperation(base, credential);
      const lease = await beginConnectionRuntimeOperation(base, credential, { expected_epoch: 0, instance_id: credential === config.hk ? "a".repeat(64) : "b".repeat(64) });
      await reportConnectionRuntimeOperation(base, credential, {
        epoch: lease.epoch, sequence: 1, revision: snapshot.payload.revision, digest: snapshot.digest, state: "applied", reason: "none",
        listeners: credential === config.sg ? [{ listener_id: "sg-udp", state: "bound", reason: "none" }] : [], peers: [peer],
      });
    }
    await page.getByRole("button", { name: "刷新概览", exact: true }).click();
    await expect(edge).toHaveAttribute("data-look", "confirmed");
    await expect(page.getByTestId("overview-status")).toContainText("1 / 1 连接双向确认");
    // A two-node ring draws a straight vertical line; open it by keyboard.
    await edge.focus();
    await page.keyboard.press("Enter");
    const channel = page.getByTestId("overview-channel");
    await expect(channel).toContainText("31 ms");
    await expect(channel).toContainText("29 ms");
    // No latency record was reported for this pair: an explicit empty state, never a drawn curve.
    await expect(page.getByTestId("latency-empty")).toContainText("这段时间没有可返回的记录");

    // Entering management keeps the selection; the map shrinks into the inset,
    // keyboard focus moves to the mode button that replaced 管理网络, and the
    // sidebar is reachable again.
    await page.getByRole("button", { name: "管理网络", exact: true }).focus();
    await page.keyboard.press("Enter");
    await expect(overview).not.toHaveAttribute("data-immersive", "true");
    await expect(page.getByRole("button", { name: "专注地图", exact: true })).toBeFocused();
    await expect(page.getByRole("button", { name: "当前网络", exact: true })).toBeVisible();
    await expect.poll(() => overviewStage(page)).toEqual({ left: 0, width: 0, top: HEADER_HEIGHT, bottom: 0, header: HEADER_HEIGHT, inset: 1440 - SIDEBAR_WIDTH, overflow: false });
    await expect(page.getByTestId("overview-selection")).toContainText(/hk-relay ⇄ sg-edge|sg-edge ⇄ hk-relay/);
    // Leaving and coming back does not replay the immersive intro.
    await page.getByRole("navigation", { name: "控制台" }).getByRole("link", { name: "路由", exact: true }).click();
    await page.getByRole("navigation", { name: "控制台" }).getByRole("link", { name: "概览", exact: true }).click();
    await expect(overview).not.toHaveAttribute("data-immersive", "true");
    await page.getByRole("button", { name: "专注地图", exact: true }).click();
    await expect(overview).toHaveAttribute("data-immersive", "true");
    // ⌘B / Ctrl+B also enters management.
    await page.keyboard.press("ControlOrMeta+b");
    await expect(overview).not.toHaveAttribute("data-immersive", "true");

    // Reduced motion: no transitions on the stage.
    await page.emulateMedia({ reducedMotion: "reduce" });
    expect(await overview.evaluate(element => getComputedStyle(element).transitionDuration)).toBe("0s");

    // A failed refresh keeps the last read only as a dated cache, visibly and with a retry.
    const status = page.getByTestId("overview-status");
    await expect(status).toContainText("更新于");
    await controller.stop();
    // The bounded background poll (12 s) reads only the runtime reports; the
    // page's own data stays as rendered.
    const failure = page.getByTestId("overview-read-error");
    await expect(failure).toContainText("运行报告刷新失败", { timeout: 25_000 });
    await expect(status).toContainText("缓存");
    await expect(status).toContainText("上次成功");
    await expect(status).not.toContainText("更新于");
    // The edge's evidence is marked as cached, with its read time.
    if (await edge.getAttribute("aria-pressed") !== "true") {
      await edge.focus();
      await page.keyboard.press("Enter");
    }
    await expect(edge).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByTestId("overview-selection")).toContainText("运行报告刷新失败");
    // Recovery: the same controller data comes back and the read is current again.
    controller = startController();
    await controller.ready(`${base}/api/v1/overview`, 401);
    await failure.getByRole("button", { name: "重试", exact: true }).click();
    await expect(failure).toHaveCount(0);
    await expect(status).toContainText("更新于");
    await expect(status).not.toContainText("缓存");

    // The overview's own read fails on a fresh load: the error is real, and
    // the management layout (navigation, retry) is there instead of a blank map.
    await controller.stop();
    await page.goto(ui.at(""));
    await expect(page.getByText("无法读取 controller", { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "重新读取", exact: true })).toBeVisible();
    await expect(page.getByRole("navigation", { name: "控制台" }).getByRole("link", { name: "路由", exact: true })).toBeVisible();
    await expect(page.locator('[data-slot="console-topbar"]')).not.toHaveAttribute("data-hidden", "true");
    expect(errors).toEqual([]);
  } finally {
    await web?.stop();
    await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
