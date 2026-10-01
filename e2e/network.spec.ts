import { expect, test } from "@playwright/test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { getNetwork, getPolicy, savePolicy } from "./network-client.ts";
import { executable, root, service, unusedPort } from "./harness.ts";

test("network identity and saved DNS intent preserve global CAS and authenticated writes", async ({ page, browser }) => {
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-network-e2e-"));
  const controllerPort = await unusedPort(); const webPort = await unusedPort();
  const base = `http://127.0.0.1:${controllerPort}`; const webUrl = `http://127.0.0.1:${webPort}`;
  const controller = service(executable("meshway-controller"), ["serve", "--data-dir", join(temporary, "controller"), "--bind", `127.0.0.1:${controllerPort}`]);
  let web: ReturnType<typeof service> | undefined;
  const anonymous = await browser.newContext(); const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await controller.ready(`${base}/api/v1/overview`, 401);
    const admin = (await readFile(join(temporary, "controller/admin.token"), "utf8")).trim();
    const original = await getNetwork(base, admin);
    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: base, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    await page.goto(`${webUrl}/network`); await expect(page).toHaveURL(/\/login(?:\?|$)/);
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).not.toHaveURL(/\/login(?:\?|$)/);
    await page.goto(`${webUrl}/network`);
    await expect(page.getByTestId("network-settings")).toBeVisible();
    await expect(page.getByTestId("network-id")).toHaveText(original.network_id);
    await expect(page.getByTestId("network-ipv4")).toHaveText(original.network.ipv4_prefix);
    await expect(page.getByTestId("network-save")).toBeDisabled();
    await page.getByTestId("network-name").fill("Fixture network");
    await page.getByTestId("network-dns-enabled").click();
    await page.getByTestId("network-dns-zone").fill("BÜCHER.TEST.");
    await page.getByTestId("network-dns-ttl").fill("17");
    const mutation = page.waitForRequest(request => request.method() === "POST" && new URL(request.url()).pathname === "/network");
    await page.getByTestId("network-save").click(); const request = await mutation;
    await expect(page.getByTestId("network-save-status")).toContainText("已保存");
    await expect(page.getByTestId("network-dns-zone")).toHaveValue("xn--bcher-kva.test");
    await expect(page.getByText("运行状态未上报", { exact: true })).toBeVisible();
    const saved = await getNetwork(base, admin);
    expect(saved.network_id).toBe(original.network_id); expect(saved.network).toEqual(original.network);
    expect(saved.dns).toEqual({ enabled: true, zone: "xn--bcher-kva.test", ttl_seconds: 17 });
    const headers = Object.fromEntries(Object.entries(await request.allHeaders()).filter(([name]) => ["content-type", "next-action", "next-router-state-tree"].includes(name.toLowerCase())));
    const body = request.postData(); if (!body) throw new Error("Missing network mutation body");
    const denied = await anonymous.request.post(request.url(), { headers: { ...headers, origin: webUrl }, data: body, maxRedirects: 0 });
    expect(denied.headers()["x-action-redirect"] ?? denied.headers().location ?? "").toContain("/login");
    expect(await getNetwork(base, admin)).toEqual(saved);

    await page.getByTestId("network-name").fill("Unsubmitted draft");
    await page.getByTestId("network-dns-ttl").fill("301");
    await expect(page.getByTestId("network-save")).toBeDisabled();
    await page.getByTestId("network-dns-ttl").fill("18");
    const oldBase = await page.getByTestId("network-base-revision").innerText();
    const policy = await getPolicy(base, admin);
    await savePolicy(base, admin, { expected_revision: policy.revision, policy: { ...policy.policy, default_action: "direct" } });
    await page.getByTestId("network-save").click();
    await expect(page.getByTestId("network-save-status")).toContainText("冲突");
    await expect(page.getByTestId("network-name")).toHaveValue("Unsubmitted draft");
    await expect(page.getByTestId("network-base-revision")).toHaveText(oldBase);
    expect((await getNetwork(base, admin)).name).toBe("Fixture network");
    await page.getByRole("button", { name: "重新读取", exact: true }).click();
    await expect(page.getByRole("alertdialog")).toBeVisible();
    await page.getByRole("button", { name: "继续编辑", exact: true }).click();
    await expect(page.getByTestId("network-name")).toHaveValue("Unsubmitted draft");
    await page.getByRole("button", { name: "重新读取", exact: true }).click();
    await page.getByRole("button", { name: "放弃草稿并重新读取", exact: true }).click();
    await expect(page.getByTestId("network-name")).toHaveValue("Fixture network");
    await page.setViewportSize({ width: 390, height: 844 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: test.info().outputPath("network-mobile.png"), fullPage: true });
    expect(errors).toEqual([]);
  } finally {
    await anonymous.close(); await web?.stop(); await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
