import { expect, test } from "@playwright/test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEnrollmentToken, getNetwork, listNodes, saveNetwork } from "./network-client.ts";
import { executable, root, run, service, unusedPort } from "./harness.ts";

test("diagnostics separates saved flow, live peers, DNS and independent netcheck", async ({ page }) => {
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-connectivity-guidance-e2e-"));
  const controllerPort = await unusedPort(); const webPort = await unusedPort();
  const base = `http://127.0.0.1:${controllerPort}`; const webUrl = `http://127.0.0.1:${webPort}`;
  const controller = service(executable("meshway-controller"), ["serve", "--data-dir", join(temporary, "controller"), "--bind", `127.0.0.1:${controllerPort}`]);
  let web: ReturnType<typeof service> | undefined;
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await controller.ready(`${base}/api/v1/overview`, 401);
    const admin = (await readFile(join(temporary, "controller/admin.token"), "utf8")).trim();
    for (const index of [1, 2]) {
      const grant = await createEnrollmentToken(base, admin, { expires_in_seconds: 600 });
      await run(executable("meshway-agent"), ["enroll", "--controller", base, "--token", grant.token, "--name", "诊断节点 'quoted'", "--state", join(temporary, `node-${index}.json`)]);
    }
    const inventory = (await listNodes(base, admin)).nodes;
    const source = inventory[0]; const target = inventory[1];
    if (!source || !target) throw new Error("Expected two real enrolled nodes");
    const original = await getNetwork(base, admin);
    const saved = await saveNetwork(base, admin, { expected_revision: original.revision, name: original.name, dns: { ...original.dns, enabled: true } });
    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: base, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    await page.goto(`${webUrl}/monitoring`);
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).not.toHaveURL(/\/login(?:\?|$)/);
    await page.goto(`${webUrl}/nodes/${source.id}?tab=connections`);
    const diagnosticLink = page.getByRole("link", { name: "诊断此节点", exact: true });
    await expect(diagnosticLink).toHaveAttribute("href", `/monitoring?source=${source.id}`);
    await diagnosticLink.click();
    await expect(page.getByRole("heading", { name: "诊断", exact: true })).toBeVisible();
    await expect(page.getByLabel("源节点（执行命令的环境）")).toHaveValue(source.id);
    await expect(page.getByLabel("目标节点（独立于源节点）")).toHaveValue("");
    await page.locator("#live-adjacency summary").focus();
    await page.keyboard.press("Enter");
    await expect(page.getByTestId("diagnostic-live-status-command")).toContainText("--live --peers --json");
    await expect(page.getByTestId("diagnostic-ping-command")).toHaveCount(0);
    await expect(page.getByRole("link", { name: "查看源节点连接运行状态" })).toHaveAttribute("href", `/nodes/${source.id}?tab=connections`);
    await page.getByLabel("目标节点（独立于源节点）").selectOption(target.id);
    const state = "--state '<源节点现有 state 文件路径>'";
    const ping = `meshway-agent ping ${target.id} ${state} --count 3 --json`;
    const whois = `meshway-agent whois ${target.id} ${state} --json`;
    await expect(page.getByTestId("diagnostic-ping-command")).toHaveText(ping);
    await expect(page.getByTestId("diagnostic-whois-command")).toHaveText(whois);
    await expect(page.getByRole("link", { name: "在拓扑中检查已保存 flow" })).toHaveAttribute("href", `/topology?src=${source.id}&dst=${target.id}#flow-check`);
    await expect(page).toHaveURL(new RegExp(`source=${source.id}&target=${target.id}`));
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
    for (const [label, value] of [["复制目标 ping 命令", ping], ["复制目标 whois 命令", whois]]) {
      const copy = page.getByRole("button", { name: label, exact: true });
      await copy.focus(); await page.keyboard.press("Enter");
      await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(value);
    }
    await page.locator("#dns summary").click();
    const fqdn = `id.${target.id.slice(0, 32)}.${target.id.slice(32)}.${saved.dns.zone}`;
    const dnsQuery = `meshway-agent dns query ${fqdn} ${state} --type A --json`;
    await expect(page.getByTestId("diagnostic-dns-query-command")).toHaveText(dnsQuery);
    await page.getByRole("button", { name: "复制目标 DNS 查询命令", exact: true }).click();
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(dnsQuery);

    // A direct hash link expands the local-only probe, without inventing observations.
    await page.goto(`${webUrl}/monitoring?source=${source.id}&target=${target.id}#netcheck`);
    const netcheck = "meshway-agent netcheck --stun-server '<NUMERIC_IPV4_A>:3478' --stun-server '<DIFFERENT_NUMERIC_IPV4_B>:3478' --bind 0.0.0.0:0 --json";
    await expect(page.getByTestId("diagnostic-netcheck-command")).toBeVisible();
    await expect(page.getByTestId("diagnostic-netcheck-command")).toHaveText(netcheck);
    await expect(page.locator("#netcheck")).toContainText("独立探测 · 未上报");
    await expect(page.locator("#netcheck")).toContainText("同 IP 的不同端口不能比较");
    await expect(page.locator("#netcheck")).toContainText("不注册候选");
    expect(netcheck).not.toContain("--state");
    await page.getByRole("button", { name: "复制 netcheck 命令模板", exact: true }).focus();
    await page.keyboard.press("Enter");
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(netcheck);
    await expect(page.getByRole("table")).toHaveCount(0);
    await expect(page.getByText("未测量：Mesh 流量", { exact: false })).toBeVisible();
    await expect(page.getByRole("link", { name: "netcheck 使用说明" })).toHaveAttribute("href", /\/netcheck$/);
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 900 });
      for (const summary of await page.locator("details:not([open]) > summary").all()) {
        if (await summary.isVisible()) await summary.click();
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await page.screenshot({ path: test.info().outputPath(`diagnostics-${width}.png`), fullPage: true });
    }

    await page.goto(`${webUrl}/monitoring?node=${source.id}&target=${source.id}#live-adjacency`);
    await expect(page.getByLabel("源节点（执行命令的环境）")).toHaveValue(source.id);
    await expect(page.getByText("源与目标相同", { exact: false })).toBeVisible();
    await expect(page.getByTestId("diagnostic-ping-command")).toHaveCount(0);
    await page.goto(`${webUrl}/monitoring?source=${"f".repeat(64)}&target=${target.id}#live-adjacency`);
    await expect(page.getByText("链接中的节点已不可用", { exact: false })).toBeVisible();
    await expect(page.getByTestId("diagnostic-whois-command")).toHaveCount(0);
    expect(errors).toEqual([]);
  } finally {
    await web?.stop(); await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
