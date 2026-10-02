import { expect, test, type Page } from "@playwright/test";
import { randomBytes } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

import { createEnrollmentToken, createNetwork, getConnections, listNodes, saveConnections } from "./network-client.ts";
import {
  beginConnectionRuntimeOperation,
  getAgentConnectionsOperation,
  queryLatencyHistoryOperation,
  reportConnectionRuntimeOperation,
  reportLatencySamplesOperation,
} from "../packages/control-client/src/generated/operations.gen.ts";
import type { LatencySample } from "../packages/control-client/src/generated/types.gen.ts";
import { consoleAt, executable, root, run, service, unusedPort } from "./harness.ts";

// Every RTT, timestamp and ID below is a fixture value reported through the
// controller's real node-authenticated report path; nothing here measures a
// real network.

async function credentialOf(state: string): Promise<string> {
  const parsed: unknown = JSON.parse(await readFile(state, "utf8"));
  if (typeof parsed !== "object" || parsed === null || !("node_credential" in parsed) || typeof parsed.node_credential !== "string") throw new Error("Missing fixture credential");
  return parsed.node_credential;
}
const hex = () => randomBytes(32).toString("hex");

async function openEdge(page: Page, pattern: RegExp) {
  const edge = page.getByRole("button", { name: pattern });
  // Two-node lens edges can be axis-aligned (zero-area box): open by keyboard.
  await edge.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("overview-selection")).toBeVisible();
}

test("the selected link shows its real directed RTT history from the controller", async ({ page }) => {
  test.setTimeout(240_000);
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-latency-e2e-"));
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
    const states: Record<string, string> = {};
    for (const name of ["hk-relay", "sg-edge", "home-gw"]) {
      const grant = await createEnrollmentToken(base, admin, { expires_in_seconds: 300 });
      states[name] = join(temporary, `${name}.json`);
      await run(executable("meshway-agent"), ["enroll", "--controller", base, "--token", grant.token, "--name", name, "--state", states[name]]);
      await run(executable("meshway-agent"), ["connections", "publish", "--state", states[name]]);
    }
    const nodes = (await listNodes(base, admin)).nodes;
    const id = (name: string) => { const node = nodes.find(entry => entry.name === name); if (!node) throw new Error(name); return node.id; };
    const [hk, sg, home] = [id("hk-relay"), id("sg-edge"), id("home-gw")];
    const plan = await getConnections(base, admin);
    await saveConnections(base, admin, { expected_revision: plan.revision, configuration: {
      listeners: [{ id: "sg-udp", node_id: sg, transport: "native_udp", bind: "127.0.0.1:45401", advertised_endpoints: [{ host: "127.0.0.1", port: 45401 }], credential_ref: null }],
      candidates: [
        { id: "hk-to-sg", dialer_node_id: hk, listener_id: "sg-udp", endpoint: { host: "127.0.0.1", port: 45401 }, priority: 1, credential_ref: null },
        { id: "home-to-sg", dialer_node_id: home, listener_id: "sg-udp", endpoint: { host: "127.0.0.1", port: 45401 }, priority: 1, credential_ref: null },
      ],
    } });

    // Applied runtime reports with authenticated peers: the controller only accepts samples on a current, permitted adjacency.
    const credential = { hk: await credentialOf(states["hk-relay"] ?? ""), sg: await credentialOf(states["sg-edge"] ?? ""), home: await credentialOf(states["home-gw"] ?? "") };
    const peer = (peerNodeId: string, initiator: string, candidate: string | null) => ({ peer_node_id: peerNodeId, transport: "native_udp" as const, initiator_node_id: initiator, listener_id: "sg-udp", candidate_id: candidate, state: "authenticated" as const, reason: "none" as const, probe_fresh: true, probe_rtt_ms: 2 });
    const lease: Record<string, { epoch: number; revision: number; digest: string }> = {};
    for (const [key, token, peers, listeners] of [
      ["hk", credential.hk, [peer(sg, hk, "hk-to-sg")], []],
      ["home", credential.home, [peer(sg, home, "home-to-sg")], []],
      ["sg", credential.sg, [peer(hk, hk, null), peer(home, home, null)], [{ listener_id: "sg-udp", state: "bound" as const, reason: "none" as const }]],
    ] as const) {
      const snapshot = await getAgentConnectionsOperation(base, token);
      const begun = await beginConnectionRuntimeOperation(base, token, { expected_epoch: 0, instance_id: hex() });
      await reportConnectionRuntimeOperation(base, token, { epoch: begun.epoch, sequence: 1, revision: snapshot.payload.revision, digest: snapshot.digest, state: "applied", reason: "none", listeners: [...listeners], peers: [...peers] });
      lease[key] = { epoch: begun.epoch, revision: snapshot.payload.revision, digest: snapshot.digest };
    }
    const started = Date.now();
    // Measurements must start after the runtime epoch; let the fixture timeline cover a few seconds.
    await delay(8_000);
    const generation = [hex(), hex()];
    const sample = (key: "hk" | "sg", peerNodeId: string, offsetMs: number, outcome: LatencySample["outcome"], rttUs: number | null, generationIndex = 0, candidate: string | null = "hk-to-sg"): LatencySample => {
      const at = started + offsetMs;
      const info = lease[key];
      if (!info) throw new Error(key);
      return {
        measurement_id: hex(), peer_node_id: peerNodeId, direction: "source_to_peer", runtime_epoch: info.epoch, revision: info.revision, digest: info.digest,
        sampler_generation: generation[generationIndex] ?? hex(), measured_at: new Date(at).toISOString(), completed_at: new Date(at + Math.ceil((rttUs ?? 2_000_000) / 1000)).toISOString(),
        outcome, failure_reasons: outcome === "failed" ? ["channel_unavailable"] : [],
        channel_associations: outcome === "success" ? [{ transport: "native_udp", initiator_node_id: hk, listener_id: "sg-udp", candidate_id: candidate }] : [],
        rtt_us: outcome === "success" ? rttUs : null,
      };
    };
    const hkSamples = [
      sample("hk", sg, 1_000, "success", 1_520),
      sample("hk", sg, 2_000, "success", 1_610),
      sample("hk", sg, 3_000, "timeout", null),
      sample("hk", sg, 4_000, "failed", null),
      sample("hk", sg, 5_000, "success", 420, 1),
    ];
    // Out of receive order: later measurements are reported first.
    await reportLatencySamplesOperation(base, credential.hk, { samples: [hkSamples[4], hkSamples[2], hkSamples[0]].filter((entry): entry is LatencySample => entry !== undefined) });
    await reportLatencySamplesOperation(base, credential.hk, { samples: [hkSamples[3], hkSamples[1]].filter((entry): entry is LatencySample => entry !== undefined) });
    await reportLatencySamplesOperation(base, credential.sg, { samples: [
      { ...sample("sg", hk, 1_500, "success", 900), channel_associations: [{ transport: "native_udp", initiator_node_id: hk, listener_id: "sg-udp", candidate_id: null }] },
      { ...sample("sg", hk, 2_500, "success", 950), channel_associations: [{ transport: "native_udp", initiator_node_id: hk, listener_id: "sg-udp", candidate_id: null }] },
    ] });

    // The real cursor pages cover everything regardless of receive order.
    const network = (await consoleAt(webUrl, base, admin)).networkId;
    const window = { source_node_id: hk, peer_node_id: sg, from: new Date(started - 60_000).toISOString(), until: new Date(Date.now() + 1_000).toISOString() };
    const first = await queryLatencyHistoryOperation(base, admin, network, { ...window, limit: 2, after: null });
    expect(first.records).toHaveLength(2);
    expect(first.next).not.toBeNull();
    const rest = await queryLatencyHistoryOperation(base, admin, network, { ...window, limit: 1000, after: first.next });
    expect(first.records.length + rest.records.length).toBe(5);
    expect([...first.records, ...rest.records].every(record => record.historical)).toBe(true);
    // Another network cannot read this pair.
    const other = await createNetwork(base, admin, { name: "Other network" });
    await expect(queryLatencyHistoryOperation(base, admin, other.network_id, { ...window, limit: 10, after: null })).rejects.toThrow(/404/);

    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: base, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    const ui = await consoleAt(webUrl, base, admin);
    await page.goto(`${webUrl}/login`);
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect.poll(() => new URL(page.url()).pathname).toBe(ui.path());

    // A delayed response for the first direction must not overwrite the second.
    let release: (() => void) | undefined;
    let held = false;
    await page.route(ui.at(""), async route => {
      const body = route.request().postData() ?? "";
      if (route.request().method() === "POST" && !held && body.includes(`"sourceNodeId":"${hk}"`)) {
        held = true;
        await new Promise<void>(resolve => { release = resolve; });
      }
      await route.continue();
    });
    await openEdge(page, /^hk-relay 与 sg-edge|^sg-edge 与 hk-relay/);
    const history = page.getByTestId("latency-history");
    await expect(history.getByTestId("latency-loading")).toBeVisible();
    await expect.poll(() => held).toBe(true);
    await history.getByRole("radio", { name: "sg-edge → hk-relay" }).or(history.getByRole("button", { name: "sg-edge → hk-relay" })).click();
    // Server actions run one at a time: the held hk → sg read completes first and must be fenced.
    await expect(history.getByTestId("latency-loading")).toBeVisible();
    await page.evaluate(() => {
      const seen: string[] = [];
      Reflect.set(window, "__latencySummaries", seen);
      new MutationObserver(() => {
        const text = document.querySelector('[data-testid="latency-summary"]')?.textContent;
        if (text) seen.push(text);
      }).observe(document.body, { subtree: true, childList: true, characterData: true });
    });
    release?.();
    await expect(history.getByTestId("latency-summary")).toContainText("成功 2");
    await expect(history.getByTestId("latency-summary")).toContainText("超时 0 · 失败 0");
    const summaries: unknown = await page.evaluate(() => Reflect.get(window, "__latencySummaries"));
    expect(Array.isArray(summaries) && summaries.every(text => typeof text === "string" && text.startsWith("成功 2"))).toBe(true);
    await page.unroute(ui.at(""));

    // hk → sg: two segments (sampler generation changed), one timeout, one failure; sub-millisecond precision.
    await history.getByRole("radio", { name: "hk-relay → sg-edge" }).or(history.getByRole("button", { name: "hk-relay → sg-edge" })).click();
    const summary = history.getByTestId("latency-summary");
    await expect(summary).toContainText("成功 3");
    await expect(summary).toContainText("中位 1.5 ms");
    await expect(summary).toContainText("0.42 ms");
    await expect(summary).toContainText("超时 1 · 失败 1");
    await expect(summary).toContainText("2 段");
    // Current RTT and history stay separate surfaces.
    await expect(page.getByTestId("overview-channel")).toBeVisible();

    await history.getByRole("button", { name: "展开延迟历史" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByTestId("latency-events")).toContainText("分段变化：采样代际");
    await expect(dialog.getByTestId("latency-events")).toContainText("探测超时");
    await expect(dialog.getByTestId("latency-events")).toContainText("探测失败：通道不可用");
    const recordsList = dialog.getByTestId("latency-records");
    await expect(recordsList.locator("li")).toHaveCount(5);
    await expect(recordsList.locator("li").first()).toContainText("1.5 ms");
    await expect(recordsList.locator("li").last()).toContainText("0.42 ms");
    // Keyboard readout of individual records.
    await dialog.getByRole("group", { name: /延迟历史图/ }).focus();
    await page.keyboard.press("Home");
    await expect(dialog.getByText(/1\.5 ms · Native UDP/)).toBeVisible();
    await page.keyboard.press("End");
    await expect(dialog.getByText(/0\.42 ms/).first()).toBeVisible();
    await dialog.getByRole("radio", { name: "15 分钟" }).or(dialog.getByRole("button", { name: "15 分钟" })).click();
    await expect(dialog.getByTestId("latency-summary")).toContainText("成功 3");

    // A failing read shows the error with the last good read time and recovers on retry.
    let fail = true;
    await page.route(ui.at(""), async route => {
      if (fail && route.request().method() === "POST" && (route.request().postData() ?? "").includes('"sourceNodeId"')) await route.abort();
      else await route.continue();
    });
    await dialog.getByRole("button", { name: "重新读取" }).click();
    await expect(dialog.getByTestId("latency-error")).toContainText("延迟历史读取失败");
    await expect(dialog.getByTestId("latency-error")).toContainText("显示");
    await expect(dialog.getByTestId("latency-summary")).toContainText("成功 3");
    fail = false;
    await dialog.getByTestId("latency-error").getByRole("button", { name: "重试" }).click();
    await expect(dialog.getByTestId("latency-error")).toHaveCount(0);
    await page.unroute(ui.at(""));
    const shots = process.env.MESHWAY_LATENCY_SCREENSHOTS;
    if (shots) {
      for (const scheme of ["light", "dark"] as const) {
        await page.emulateMedia({ colorScheme: scheme });
        for (const width of [1440, 390]) {
          await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
          await page.keyboard.press("End");
          await page.screenshot({ path: join(shots, `${scheme}-${width}-expanded.png`) });
        }
      }
      await page.setViewportSize({ width: 1440, height: 900 });
    }
    await page.keyboard.press("Escape");
    if (shots) {
      for (const scheme of ["light", "dark"] as const) {
        await page.emulateMedia({ colorScheme: scheme });
        for (const width of [1440, 390]) {
          await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
          await page.screenshot({ path: join(shots, `${scheme}-${width}-edge.png`) });
        }
      }
      await page.emulateMedia({ colorScheme: "light" });
      await page.setViewportSize({ width: 1440, height: 900 });
    }

    // A configured link without any record: an explicit empty state, never zero.
    await page.getByRole("button", { name: "关闭" }).click();
    await openEdge(page, /^home-gw 与 sg-edge|^sg-edge 与 home-gw/);
    await expect(page.getByTestId("latency-empty")).toContainText("这段时间没有记录");
    await expect(page.getByTestId("latency-empty")).toContainText("不代表断开");
    expect(errors).toEqual([]);
  } finally {
    await web?.stop();
    await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
