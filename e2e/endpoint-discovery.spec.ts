import { expect, test } from "@playwright/test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEnrollmentToken, getConnections, getEndpointDiscoveries, listNodes, saveConnections } from "./network-client.ts";
import { beginConnectionRuntimeOperation, getAgentConnectionsOperation, reportConnectionRuntimeOperation, reportEndpointDiscoveryOperation } from "../packages/control-client/src/generated/operations.gen.ts";
import { executable, expandDetails, root, run, service, unusedPort, consoleAt, expectNoOverflow } from "./harness.ts";

test("node-authenticated fixture self-reports display as leased unverified carrier candidates", async ({ page }) => {
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-discovery-web-e2e-"));
  const controllerPort = await unusedPort(); const webPort = await unusedPort();
  const base = `http://127.0.0.1:${controllerPort}`; const webUrl = `http://127.0.0.1:${webPort}`;
  const controller = service(executable("meshway-controller"), ["serve", "--data-dir", join(temporary, "controller"), "--bind", `127.0.0.1:${controllerPort}`]);
  let web: ReturnType<typeof service> | undefined;
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await controller.ready(`${base}/api/v1/overview`, 401);
    const admin = (await readFile(join(temporary, "controller/admin.token"), "utf8")).trim();
    for (const name of ["fixture-carrier", "fixture-awaiting"]) {
      const grant = await createEnrollmentToken(base, admin, { expires_in_seconds: 600 });
      const state = join(temporary, `${name}.json`);
      await run(executable("meshway-agent"), ["enroll", "--controller", base, "--token", grant.token, "--name", name, "--state", state]);
      await run(executable("meshway-agent"), ["connections", "publish", "--state", state]);
    }
    const nodes = (await listNodes(base, admin)).nodes;
    const carrier = nodes.find(node => node.name === "fixture-carrier");
    const awaiting = nodes.find(node => node.name === "fixture-awaiting");
    if (!carrier || !awaiting) throw new Error("Missing enrolled fixture nodes");
    const privateState: unknown = JSON.parse(await readFile(join(temporary, "fixture-carrier.json"), "utf8"));
    if (typeof privateState !== "object" || privateState === null || !("node_credential" in privateState) || typeof privateState.node_credential !== "string") throw new Error("Missing private fixture node credential");
    const credential = privateState.node_credential;
    const original = await getConnections(base, admin);
    await saveConnections(base, admin, { expected_revision: original.revision, configuration: {
      ...original.configuration,
      listeners: [{ id: "fixture-carrier-udp", node_id: carrier.id, transport: "native_udp", bind: "[::]:45123", advertised_endpoints: [{ host: "127.0.0.1", port: 45123 }], credential_ref: null }],
    } });
    const config = await getAgentConnectionsOperation(base, credential);
    const lease = await beginConnectionRuntimeOperation(base, credential, { expected_epoch: 0, instance_id: "a".repeat(64) });
    await reportConnectionRuntimeOperation(base, credential, {
      epoch: lease.epoch, sequence: 1, revision: config.payload.revision, digest: config.digest,
      state: "applied", reason: "none", listeners: [{ listener_id: "fixture-carrier-udp", state: "bound", reason: "none" }], peers: [],
    });
    // This fixture submits a node-authenticated self-report. It does not run STUN
    // or assert that an advertised / observed endpoint is reachable or connected.
    const observedAtMs = Date.now();
    const observedAt = new Date(observedAtMs).toISOString();
    const expiresAt = new Date(observedAtMs + 30_000).toISOString();
    const mappedEndpoint = "[2001:db8:1234:5678:abcd:ef01:2345:6789]:45123";
    await reportEndpointDiscoveryOperation(base, credential, {
      epoch: lease.epoch, sequence: 1, revision: config.payload.revision, digest: config.digest, state: "observed",
      sockets: [{ resource_kind: "listener", resource_id: "fixture-carrier-udp", generation: 1, socket_id: 1, bound_address: "[::]:45123", source: "managed_udp_socket_stun",
        observations: [{ observer: "[2001:db8::1]:3478", outcome: "mapped", mapped_endpoint: mappedEndpoint, rtt_ms: 12, observed_at: observedAt, expires_at: expiresAt }],
      }],
    });
    expect((await getEndpointDiscoveries(base, admin)).nodes.find(node => node.node_id === carrier.id)?.sockets).toHaveLength(1);
    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: base, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    const ui = await consoleAt(webUrl, base, admin);
    await page.goto(`${ui.at("/diagnostics")}?source=${carrier.id}#netcheck`);
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect.poll(() => new URL(page.url()).pathname).toBe(ui.path());
    await page.goto(`${ui.at("/diagnostics")}?source=${carrier.id}#netcheck`);
    const panel = page.getByTestId("endpoint-discovery");
    await expect(panel.getByTestId("discovery-status")).toHaveText("未验证候选观测");
    await expect(panel).toContainText(mappedEndpoint);
    await expandDetails(panel, "承载 socket 技术详情");
    await expect(panel).toContainText("generation 1 / socket 1");
    await expect(panel).toContainText("managed_udp_socket_stun");
    await expect(panel.getByRole("link", { name: "节点连接配置与运行状态" })).toHaveAttribute("href", `${ui.path("/machines")}/${carrier.id}?tab=connections`);
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
    await panel.getByRole("button", { name: "复制观测 IP:port", exact: true }).focus();
    await page.keyboard.press("Enter");
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(mappedEndpoint);
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 900 });
      await expectNoOverflow(page);
      await page.screenshot({ path: test.info().outputPath(`carrier-discovery-${width}.png`), fullPage: true });
    }
    // Advance only the browser clock, then let the one-second lease tick run.
    // Controller still retains the report, proving local expiry independently.
    await page.clock.install();
    await page.clock.setSystemTime(new Date(Date.parse(expiresAt) + 1));
    await page.clock.runFor(1_000);
    await expect(panel.getByTestId("discovery-status")).toHaveText("观测已失效");
    await expect(panel.getByRole("button", { name: "复制历史观测 IP:port（失效）", exact: true })).toBeDisabled();
    await page.clock.setSystemTime(new Date(Date.parse(expiresAt) - 10_000));
    await page.clock.runFor(1_000);
    await expect(panel.getByTestId("discovery-status")).toHaveText("观测已失效");
    await expect(panel.getByRole("button", { name: "复制历史观测 IP:port（失效）", exact: true })).toBeDisabled();
    await page.goto(`${ui.at("/machines")}/${awaiting.id}?tab=connections`);
    await expandDetails(page, "UDP 地址观测与当前启用帮助");
    await expect(page.getByTestId("discovery-status")).toHaveText("尚无报告");
    await expect(page.getByTestId("endpoint-discovery")).not.toContainText(mappedEndpoint);
    await expandDetails(page, "当前 STUN 启用帮助");
    await expect(page.getByTestId("endpoint-discovery")).toContainText("不另起第二个 daemon");
    await page.goto(`${ui.at("/machines")}/${carrier.id}?tab=connections`);
    await expandDetails(page, "UDP 地址观测与当前启用帮助");
    await expect(page.getByTestId("endpoint-discovery")).toContainText("fixture-carrier-udp");
    // Only this isolated fixture process is stopped. A failed authenticated read
    // must replace all last-good observation values with a real retryable error.
    await controller.stop();
    await page.getByRole("button", { name: "刷新观测", exact: true }).click();
    await expect(page.getByTestId("endpoint-discovery").getByRole("alert")).toContainText("端点观测读取失败");
    await expect(page.getByTestId("endpoint-discovery")).not.toContainText(mappedEndpoint);
    await expect(page.getByRole("button", { name: "刷新观测", exact: true })).toBeEnabled();
    expect(errors).toEqual([]);
  } finally {
    await web?.stop(); await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
