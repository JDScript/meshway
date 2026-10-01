import { expect, test } from "@playwright/test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEnrollmentToken, getNetwork, getPolicy, listNodes, revokeNode, savePolicy } from "./network-client.ts";
import { executable, expandDetails, root, run, service, unusedPort, consoleAt } from "./harness.ts";

test("DNS guidance uses saved intent and full identity without claiming local runtime", async ({ page }) => {
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-dns-guidance-e2e-"));
  const controllerPort = await unusedPort(); const webPort = await unusedPort();
  const base = `http://127.0.0.1:${controllerPort}`; const webUrl = `http://127.0.0.1:${webPort}`;
  const controller = service(executable("meshway-controller"), ["serve", "--data-dir", join(temporary, "controller"), "--bind", `127.0.0.1:${controllerPort}`]);
  let web: ReturnType<typeof service> | undefined;
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  const nodePage = await page.context().newPage();
  nodePage.on("pageerror", error => errors.push(error.message));
  try {
    await controller.ready(`${base}/api/v1/overview`, 401);
    const admin = (await readFile(join(temporary, "controller/admin.token"), "utf8")).trim();
    // Real keyless nodes with an ambiguous display-name alias. UI must not guess
    // a readable alias even when one node is displayed in isolation.
    for (const index of [1, 2]) {
      const grant = await createEnrollmentToken(base, admin, { expires_in_seconds: 600 });
      await run(executable("meshway-agent"), ["enroll", "--controller", base, "--token", grant.token, "--name", "重复名称", "--state", join(temporary, `node-${index}.json`)]);
    }
    const node = (await listNodes(base, admin)).nodes[0];
    if (!node) throw new Error("Expected a real enrolled node");
    const original = await getNetwork(base, admin);
    const nameFor = (zone: string) => `id.${node.id.slice(0, 32)}.${node.id.slice(32)}.${zone}`;
    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: base, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    const ui = await consoleAt(webUrl, base, admin);
    await page.goto(ui.at("/dns"));
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).not.toHaveURL(/\/login(?:\?|$)/);
    await page.goto(ui.at("/dns"));
    await expect(page.getByTestId("network-dns-saved-context")).toContainText("DNS 已关闭");
    await expandDetails(page, "DNS 诊断与当前启用帮助");
    await expect(page.getByTestId("dns-status-command")).toHaveText("meshway-agent dns status --state '<该节点现有 state 文件路径>'");

    await nodePage.goto(ui.at(`/machines/${node.id}`));
    await expect(nodePage.getByTestId("node-dns-disabled")).toBeVisible();
    await expect(nodePage.getByTestId("node-dns-canonical")).toHaveText(nameFor(original.dns.zone));
    await expect(nodePage.getByTestId("node-dns-query-command")).toHaveCount(0);

    await page.getByTestId("network-dns-enabled").click();
    await page.getByTestId("network-dns-zone").fill("BÜCHER.TEST.");
    await expect(page.getByTestId("network-dns-saved-context")).toContainText(original.dns.zone);
    await expect(page.getByTestId("network-dns-saved-context")).toContainText("DNS 已关闭");
    await page.getByTestId("network-save").click();
    await expect(page.getByTestId("network-save-status")).toContainText("已保存");
    const saved = await getNetwork(base, admin);
    expect(saved.dns.zone).toBe("xn--bcher-kva.test");
    await expect(page.getByTestId("network-dns-saved-context")).toContainText("尚不能证明本地解析服务运行");

    await nodePage.reload();
    const fqdn = nameFor(saved.dns.zone);
    const dig = `dig @127.0.0.1 -p 5353 ${fqdn}. A`;
    await expect(nodePage.getByTestId("node-dns-canonical")).toHaveText(fqdn);
    await expect(nodePage.getByTestId("node-dns-query-command")).toHaveText(dig);
    await expect(nodePage.getByText("本地 DNS listener 与系统 resolver 状态未知。", { exact: false })).toBeVisible();
    await expect(nodePage.getByRole("link", { name: "配置与诊断", exact: true })).toHaveAttribute("href", ui.path("/dns#dns-diagnostics"));
    await expect(nodePage.getByRole("link", { name: "DNS 配置与诊断" })).toHaveAttribute("href", /\/dns$/);
    await expect(nodePage.getByRole("link", { name: "Agent CLI", exact: true })).toHaveAttribute("href", /\/cli$/);

    await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
    await nodePage.bringToFront();
    const copyName = nodePage.getByRole("button", { name: "复制完整 DNS FQDN", exact: true });
    await copyName.focus(); await nodePage.keyboard.press("Enter");
    await expect.poll(() => nodePage.evaluate(() => navigator.clipboard.readText())).toBe(fqdn);
    const copyDig = nodePage.getByRole("button", { name: "复制节点 DNS 查询命令", exact: true });
    await copyDig.focus(); await nodePage.keyboard.press("Enter");
    await expect.poll(() => nodePage.evaluate(() => navigator.clipboard.readText())).toBe(dig);

    // The help uses the editing base, including during a global CAS conflict.
    await page.getByTestId("network-dns-zone").fill("unsaved.example");
    const oldBase = await page.getByTestId("network-base-revision").innerText();
    const policy = await getPolicy(base, admin);
    await savePolicy(base, admin, { expected_revision: policy.revision, policy: { ...policy.policy, default_action: "direct" } });
    await page.getByTestId("network-save").click();
    await expect(page.getByTestId("network-save-status")).toContainText("冲突");
    await expect(page.getByTestId("network-dns-zone")).toHaveValue("unsaved.example");
    await expect(page.getByTestId("network-base-revision")).toHaveText(oldBase);
    await expect(page.getByTestId("network-dns-saved-context")).toContainText(saved.dns.zone);
    await expect(page.getByTestId("network-dns-saved-context")).not.toContainText("unsaved.example");
    await page.getByRole("button", { name: "重新读取", exact: true }).click();
    await page.getByRole("button", { name: "继续编辑", exact: true }).click();
    await expect(page.getByTestId("network-dns-zone")).toHaveValue("unsaved.example");

    for (const [surface, filename] of [[page, "network-dns-guidance-mobile.png"], [nodePage, "node-dns-guidance-mobile.png"]] as const) {
      await surface.setViewportSize({ width: 390, height: 844 });
      expect(await surface.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await surface.screenshot({ path: test.info().outputPath(filename), fullPage: true });
    }
    const box = await nodePage.getByTestId("node-dns-canonical").boundingBox();
    expect(box).not.toBeNull();
    if (box) { expect(box.x).toBeGreaterThanOrEqual(0); expect(box.x + box.width).toBeLessThanOrEqual(390); }
    await test.info().attach("canonical-mobile-bbox", { body: JSON.stringify(box), contentType: "application/json" });
    await revokeNode(base, admin, { node_id: node.id });
    await nodePage.reload();
    await expect(nodePage.getByTestId("node-dns-query-command")).toHaveCount(0);
    await expect(nodePage.getByText("节点已撤销，不发布其 DNS 记录；以下仅是身份推导名称。", { exact: true })).toBeVisible();
    expect(errors).toEqual([]);
  } finally {
    await nodePage.close(); await web?.stop(); await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
