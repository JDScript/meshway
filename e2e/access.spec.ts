import { expect, test, type Locator, type Page } from "@playwright/test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  createEnrollmentToken, getMeshAccess, getPolicy, listNodes, revokeNode, setNodeCountry,
} from "../packages/control-client/src/index.ts";
import { executable, root, run, service, unusedPort } from "./harness.ts";

async function choose(page: Page, trigger: Locator, name: RegExp) {
  await trigger.click();
  await expect(trigger).toHaveAttribute("aria-expanded", "true");
  const popup = await trigger.getAttribute("aria-controls");
  if (!popup) throw new Error("Access selector did not identify its popup");
  await page.locator(`[id=${JSON.stringify(popup)}]`).getByRole("option", { name }).click();
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
}

test("directional access editing, authenticated save and global CAS cross the real stack", async ({ page, browser }) => {
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-access-e2e-"));
  const controllerPort = await unusedPort();
  const webPort = await unusedPort();
  const controllerUrl = `http://127.0.0.1:${controllerPort}`;
  const webUrl = `http://127.0.0.1:${webPort}`;
  const controller = service(executable("meshway-controller"), ["serve", "--data-dir", join(temporary, "controller"), "--bind", `127.0.0.1:${controllerPort}`]);
  let web: ReturnType<typeof service> | undefined;
  const anonymous = await browser.newContext();
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await controller.ready(`${controllerUrl}/api/v1/overview`, 401);
    const admin = (await readFile(join(temporary, "controller/admin.token"), "utf8")).trim();
    for (const name of ["nyc", "lax", "sha", "pending-key"]) {
      const enrollment = await createEnrollmentToken(controllerUrl, admin, { expires_in_seconds: 300 });
      const state = join(temporary, `${name}.json`);
      await run(executable("meshway-agent"), ["enroll", "--controller", controllerUrl, "--token", enrollment.token, "--name", name, "--state", state]);
      if (name !== "pending-key") await run(executable("meshway-agent"), ["connections", "publish", "--state", state]);
    }
    const nodes = (await listNodes(controllerUrl, admin)).nodes;
    const nyc = nodes.find(node => node.name === "nyc");
    const lax = nodes.find(node => node.name === "lax");
    const sha = nodes.find(node => node.name === "sha");
    if (!nyc || !lax || !sha) throw new Error("Expected enrolled fixture identities");
    expect((await getMeshAccess(controllerUrl, admin)).policy.grants).toEqual([]);
    const routing = await getPolicy(controllerUrl, admin);
    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: controllerUrl, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    await page.goto(`${webUrl}/access`);
    await expect(page).toHaveURL(/\/login(?:\?|$)/);
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).not.toHaveURL(/\/login(?:\?|$)/);
    await page.goto(`${webUrl}/access`);
    await expect(page.getByTestId("access-editor")).toBeVisible();
    await expect(page.getByTestId("access-grant-row")).toHaveCount(0);
    await expect(page.getByTestId("access-save")).toBeDisabled();

    await test.step("a forward grant preserves full order and never silently adds its reverse", async () => {
      await page.getByTestId("access-add-grant").click();
      const builder = page.getByTestId("grant-builder");
      await expect(builder.getByTestId("grant-add-reverse")).not.toBeChecked();
      await builder.getByTestId("grant-source").click();
      await expect(page.getByRole("option", { name: /pending-key/ })).toHaveAttribute("aria-disabled", "true");
      await page.getByRole("option", { name: /^nyc/ }).click();
      await builder.getByTestId("grant-add-via").click();
      await choose(page, builder.getByTestId("grant-via-0"), /^lax/);
      await choose(page, builder.getByTestId("grant-destination"), /^sha/);
      await builder.getByTestId("grant-apply").click();
      await expect(builder).not.toBeVisible();
      expect((await getMeshAccess(controllerUrl, admin)).policy.grants).toEqual([]);
      const mutation = page.waitForRequest(request => request.method() === "POST" && new URL(request.url()).pathname === "/access");
      await page.getByTestId("access-save").click();
      const request = await mutation;
      await expect(page.getByTestId("access-save-status")).toContainText("已保存");
      await expect(page.getByTestId("access-save")).toBeDisabled();
      const saved = await getMeshAccess(controllerUrl, admin);
      expect(saved.policy.grants.map(grant => grant.path)).toEqual([[nyc.id, lax.id, sha.id]]);
      expect((await getPolicy(controllerUrl, admin)).policy).toEqual(routing.policy);
      const headers = Object.fromEntries(Object.entries(await request.allHeaders()).filter(([name]) => ["content-type", "next-action", "next-router-state-tree"].includes(name.toLowerCase())));
      const body = request.postData();
      if (!body) throw new Error("Expected access mutation body");
      const response = await anonymous.request.post(request.url(), { headers: { ...headers, origin: webUrl }, data: body, maxRedirects: 0 });
      expect(response.headers()["x-action-redirect"] ?? response.headers().location ?? "").toContain("/login");
      expect(await getMeshAccess(controllerUrl, admin)).toEqual(saved);
    });

    await test.step("explicit reverse remains a draft through navigation and CAS conflict", async () => {
      const base = await page.getByTestId("access-base-revision").innerText();
      await page.getByTestId("access-grant-row").getByRole("button", { name: "更多操作", exact: true }).click();
      await page.getByRole("menuitem", { name: "添加反向授权", exact: true }).click();
      await expect(page.getByTestId("access-grant-row")).toHaveCount(2);
      await page.getByRole("link", { name: "节点", exact: true }).first().click();
      await expect(page).toHaveURL(`${webUrl}/nodes`);
      const latest = await getMeshAccess(controllerUrl, admin);
      await setNodeCountry(controllerUrl, admin, { node_id: nyc.id, country: "US", expected_revision: latest.revision });
      await page.goBack();
      await expect(page.getByTestId("access-base-revision")).toHaveText(base);
      await expect(page.getByTestId("access-grant-row")).toHaveCount(2);
      await page.getByTestId("access-save").click();
      await expect(page.getByTestId("access-save-status")).toContainText("冲突");
      await expect(page.getByTestId("access-base-revision")).toHaveText(base);
      expect((await getMeshAccess(controllerUrl, admin)).policy.grants).toHaveLength(1);
      await page.getByRole("button", { name: "重新读取", exact: true }).click();
      await page.getByRole("alertdialog").getByRole("button", { name: "放弃草稿并重新读取", exact: true }).click();
      await expect(page.getByTestId("access-grant-row")).toHaveCount(1);
      await page.getByTestId("access-grant-row").getByRole("button", { name: "更多操作", exact: true }).click();
      await page.getByRole("menuitem", { name: "添加反向授权", exact: true }).click();
      await page.getByTestId("access-save").click();
      await expect(page.getByTestId("access-save-status")).toContainText("已保存");
      expect((await getMeshAccess(controllerUrl, admin)).policy.grants.map(grant => grant.path)).toEqual([
        [nyc.id, lax.id, sha.id], [sha.id, lax.id, nyc.id],
      ]);
      expect((await getPolicy(controllerUrl, admin)).policy).toEqual(routing.policy);
      expect((await page.content()).includes(admin)).toBe(false);
      await page.screenshot({ path: test.info().outputPath("access-desktop.png"), fullPage: true });
    });

    await test.step("revocation keeps repairable grants and the mobile view does not imply runtime success", async () => {
      await revokeNode(controllerUrl, admin, { node_id: lax.id });
      await page.reload();
      await expect(page.getByTestId("access-base-invalid")).toBeVisible();
      await expect(page.getByTestId("access-grant-row")).toHaveCount(2);
      await expect(page.getByTestId("access-grant-problems").first()).toBeVisible();
      await page.setViewportSize({ width: 390, height: 844 });
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await page.screenshot({ path: test.info().outputPath("access-mobile-invalid.png"), fullPage: true });
    });
    expect(errors).toEqual([]);
  } finally {
    await anonymous.close();
    await web?.stop();
    await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
