import { expect, test, type Page } from "@playwright/test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createEnrollmentToken, getConnections, getMeshAccess, getPolicy, savePolicy } from "../packages/control-client/src/index.ts";
import { executable, root, run, service, unusedPort } from "./harness.ts";
import { revealAdvanced } from "./web-actions.ts";

const barSelector = '[data-slot="draft-action-bar"]';

async function geometry(page: Page) {
  return page.locator('[data-slot="page-header"]').evaluate(header => {
    const inset = header.closest('[data-slot="sidebar-inset"]');
    const body = header.closest('[data-slot="page-body"]');
    if (!inset || !body) throw new Error("Missing console layout");
    const h = header.getBoundingClientRect();
    const i = inset.getBoundingClientRect();
    const b = body.getBoundingClientRect();
    return { left: h.left, right: h.right, top: h.top - b.top, leftGutter: h.left - i.left, rightGutter: i.right - h.right };
  });
}

test("console gutters and animated draft actions remain consistent across widths and sidebar states", async ({ page }) => {
  test.setTimeout(180_000);
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-layout-e2e-"));
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
    const enrollment = await createEnrollmentToken(base, admin, { expires_in_seconds: 300 });
    const state = join(temporary, "layout-node.json");
    await run(executable("meshway-agent"), ["enroll", "--controller", base, "--token", enrollment.token, "--name", "layout-node", "--state", state]);
    await run(executable("meshway-agent"), ["connections", "publish", "--state", state]);
    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: base, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    await page.goto(`${webUrl}/access`);
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).not.toHaveURL(/\/login(?:\?|$)/);

    for (const width of [1440, 1920, 390]) {
      await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
      for (const expanded of [true, false]) {
        await page.goto(webUrl);
        if (width >= 768) {
          const sidebar = page.locator('[data-slot="sidebar"]').first();
          if ((await sidebar.getAttribute("data-state")) !== (expanded ? "expanded" : "collapsed")) {
            await page.getByRole("button", { name: "切换侧边栏", exact: true }).click();
          }
          await expect.poll(() => page.locator('[data-slot="sidebar-gap"]').evaluate(element => Math.round(element.getBoundingClientRect().width))).toBe(expanded ? 256 : 64);
        }
        let edges: { left: number; right: number } | undefined;
        for (const path of ["/", "/nodes", "/access", "/routing", "/connections", "/topology"]) {
          await page.goto(`${webUrl}${path}`);
          await expect(page.locator('[data-slot="page-header"]')).toBeVisible();
          if (width === 390 && expanded) {
            await page.getByRole("button", { name: "切换侧边栏", exact: true }).click();
            await expect(page.locator('[data-slot="sidebar"][data-mobile="true"]')).toBeVisible();
          }
          const box = await geometry(page);
          expect(box.leftGutter).toBe(width === 390 ? 16 : 24);
          expect(box.rightGutter).toBe(width === 390 ? 16 : 24);
          expect(box.top).toBe(width === 390 ? 24 : 32);
          if (edges) {
            expect(box.left).toBeCloseTo(edges.left, 0);
            expect(box.right).toBeCloseTo(edges.right, 0);
          }
          edges = box;
          if (width === 390 && expanded) await page.keyboard.press("Escape");
          expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
          if (path === "/access" && process.env.MESHWAY_LAYOUT_SCREENSHOTS) {
            await page.screenshot({ path: join(process.env.MESHWAY_LAYOUT_SCREENSHOTS, `layout-gutters-${width}-${expanded ? "expanded" : "collapsed"}.png`) });
          }
        }
      }
    }

    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
      await page.goto(`${webUrl}/access`);
      const bar = page.locator(barSelector);
      await expect(bar).toBeHidden();
      expect((await bar.boundingBox())?.height ?? 0).toBe(0);
      expect(await bar.evaluate(element => {
        const previous = element.previousElementSibling;
        const editor = element.parentElement;
        if (!previous || !editor) throw new Error("Missing editor layout");
        return editor.getBoundingClientRect().bottom - previous.getBoundingClientRect().bottom;
      })).toBe(0);
      await page.getByRole("button", { name: "JSONC", exact: true }).click();
      const editor = page.getByRole("textbox", { name: "访问授权 JSONC", exact: true });
      const snapshot = await getMeshAccess(base, admin);
      const source = `// layout draft ${width}\n${JSON.stringify(snapshot.policy, null, 2)}\n`;
      await editor.fill(source);
      await expect(bar).toHaveAttribute("data-open", "true");
      await expect.poll(() => bar.evaluate(element => getComputedStyle(element).opacity)).toBe("1");
      await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
      const save = page.getByTestId("access-save");
      const before = await save.boundingBox();
      if (!before) throw new Error("Missing save button");
      expect(before.y + before.height).toBeLessThan((width === 390 ? 844 : 1000) - 12);
      expect(before.x + before.width).toBeLessThanOrEqual(width - 16);
      expect(await bar.evaluate(element => {
        const previous = element.previousElementSibling;
        if (!previous) throw new Error("Missing editor content");
        return element.getBoundingClientRect().top - previous.getBoundingClientRect().bottom;
      })).toBeGreaterThanOrEqual(16);
      let release!: () => void;
      const gate = new Promise<void>(resolve => { release = resolve; });
      await page.route(`${webUrl}/access`, async route => {
        if (route.request().method() === "POST") await gate;
        await route.continue();
      });
      await save.click();
      await expect(save).toBeDisabled();
      const during = await save.boundingBox();
      expect(during?.x).toBe(before.x);
      expect(during?.width).toBe(before.width);
      release();
      await expect(page.getByTestId("access-save-status")).toContainText("已保存");
      await page.unroute(`${webUrl}/access`);
      const after = await save.boundingBox();
      expect(after?.x).toBe(before.x);
      expect(after?.width).toBe(before.width);
      expect(after?.y).toBe(before.y);
      await expect(bar).toBeVisible();
      expect((await getMeshAccess(base, admin)).source_jsonc).toBe(source);
      if (process.env.MESHWAY_LAYOUT_SCREENSHOTS) {
        await expect(bar.getByRole("button", { name: "放弃修改", exact: true })).toBeHidden();
        await page.screenshot({ path: join(process.env.MESHWAY_LAYOUT_SCREENSHOTS, `layout-saved-${width}.png`) });
      }

      // A genuine CAS failure remains visible and keeps the draft/base intact.
      await editor.fill(`// conflict\n${source}`);
      const policy = await getPolicy(base, admin);
      await savePolicy(base, admin, { expected_revision: policy.revision, policy: { ...policy.policy, default_action: policy.policy.default_action === "reject" ? "direct" : "reject" } });
      await save.click();
      await expect(page.getByTestId("access-save-status")).toContainText("冲突");
      await expect(bar).toBeVisible();
      await expect(editor).toContainText("// conflict");
      await page.getByRole("button", { name: "放弃修改", exact: true }).click();
      await page.getByRole("alertdialog").getByRole("button", { name: "放弃修改", exact: true }).click();
      await expect(bar).toHaveAttribute("data-open", "false");
      await expect.poll(() => bar.evaluate(element => getComputedStyle(element).visibility)).toBe("hidden");
      expect((await bar.boundingBox())?.height ?? 0).toBe(0);
    }

    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.goto(`${webUrl}/routing`);
    const routingBar = page.locator(barSelector);
    await expect(routingBar).toBeHidden();
    await page.getByRole("button", { name: "JSONC", exact: true }).click();
    await page.getByRole("textbox", { name: "路由策略 JSONC", exact: true }).fill(`// reduced motion\n${JSON.stringify((await getPolicy(base, admin)).policy)}`);
    await expect(routingBar).toBeVisible();
    expect(await routingBar.evaluate(element => getComputedStyle(element).transitionDuration)).toBe("0s");
    expect(await routingBar.evaluate(element => getComputedStyle(element).transform)).toBe("none");
    await page.getByRole("button", { name: "保存策略", exact: true }).click();
    await expect(page.getByTestId("policy-save-status")).toContainText("已保存");

    await page.goto(`${webUrl}/connections`);
    const connectionsBar = page.locator(barSelector);
    await expect(connectionsBar).toBeHidden();
    await page.getByRole("button", { name: "接收其他节点连接", exact: true }).first().click();
    const sheet = page.getByRole("dialog");
    await revealAdvanced(sheet, sheet.getByLabel("监听器 ID", { exact: true }));
    await sheet.getByLabel("监听器 ID", { exact: true }).fill("layout-listener");
    const picker = sheet.getByLabel("监听节点", { exact: true });
    await picker.click();
    await expect(picker).toHaveAttribute("aria-expanded", "true");
    const popup = await picker.getAttribute("aria-controls");
    if (!popup) throw new Error("Missing node picker listbox");
    await page.locator(`[id=${JSON.stringify(popup)}]`).getByRole("option", { name: /^layout-node/ }).click();
    await sheet.getByLabel("监听端口", { exact: true }).fill("42081");
    await sheet.getByLabel("对外地址 1 主机", { exact: true }).fill("127.0.0.1");
    await sheet.getByLabel("对外地址 1 端口", { exact: true }).fill("42081");
    await expect(connectionsBar).toBeHidden();
    expect((await getConnections(base, admin)).configuration.listeners).toHaveLength(0);
    await sheet.getByRole("button", { name: "添加到草稿", exact: true }).click();
    await expect(sheet).not.toBeVisible();
    await expect(connectionsBar).toBeVisible();
    await page.getByRole("button", { name: "保存连接配置", exact: true }).click();
    await expect(page.getByTestId("connections-save-status")).toContainText("已保存");
    expect((await getConnections(base, admin)).configuration.listeners).toHaveLength(1);
    expect(errors).toEqual([]);
  } finally {
    await web?.stop();
    await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
