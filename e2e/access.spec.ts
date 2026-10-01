import { expect, test } from "@playwright/test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  createEnrollmentToken, getMeshAccess, getPolicy, getTags, listNodes,
  previewFlow, revokeNode, savePolicy, setNodeCountry,
} from "./network-client.ts";
import { consoleAt, executable, root, run, service, unusedPort, expectNoOverflow } from "./harness.ts";

test("tagged endpoint access, JSONC drafts, first-match and CAS cross the real stack", async ({ page, browser }) => {
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-access-v2-e2e-"));
  const controllerPort = await unusedPort(); const webPort = await unusedPort();
  const base = `http://127.0.0.1:${controllerPort}`; const webUrl = `http://127.0.0.1:${webPort}`;
  const controller = service(executable("meshway-controller"), ["serve", "--data-dir", join(temporary, "controller"), "--bind", `127.0.0.1:${controllerPort}`]);
  let web: ReturnType<typeof service> | undefined;
  const anonymous = await browser.newContext(); const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await controller.ready(`${base}/api/v1/overview`, 401);
    const admin = (await readFile(join(temporary, "controller/admin.token"), "utf8")).trim();
    for (const name of ["nyc", "sha", "pending-key"]) {
      const enrollment = await createEnrollmentToken(base, admin, { expires_in_seconds: 300 });
      const state = join(temporary, `${name}.json`);
      await run(executable("meshway-agent"), ["enroll", "--controller", base, "--token", enrollment.token, "--name", name, "--state", state]);
      if (name !== "pending-key") await run(executable("meshway-agent"), ["connections", "publish", "--state", state]);
    }
    const nodes = (await listNodes(base, admin)).nodes;
    const nyc = nodes.find(node => node.name === "nyc"); const sha = nodes.find(node => node.name === "sha");
    if (!nyc || !sha) throw new Error("Expected real enrolled fixture identities");
    const policy = await getPolicy(base, admin);
    await savePolicy(base, admin, { expected_revision: policy.revision, policy: { default_action: "direct", rules: [] } });
    expect((await getMeshAccess(base, admin)).policy).toEqual({ default_action: "reject", rules: [] });
    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: base, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    const ui = await consoleAt(webUrl, base, admin);
    await page.goto(ui.at("/access")); await expect(page).toHaveURL(/\/login(?:\?|$)/);
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).not.toHaveURL(/\/login(?:\?|$)/);

    await test.step("create tags and assign members without public-key prerequisites", async () => {
      await page.goto(ui.at("/tags"));
      for (const tag of ["client", "server"]) {
        await page.getByTestId("tag-new-name").fill(tag);
        await page.getByTestId("tag-add").click();
      }
      await page.getByTestId("tag-catalog-save").click();
      await expect(page.getByTestId("tags-save-status")).toContainText("已保存");
      for (const [name, tag] of [["nyc", "client"], ["sha", "server"], ["pending-key", "client"]]) {
        const row = page.getByTestId("node-tags-row").filter({ has: page.getByRole("link", { name, exact: true }) });
        await row.getByRole("button", { name: `编辑 ${name} 的标签`, exact: true }).click();
        await row.getByRole("checkbox", { name: tag, exact: true }).check();
        await row.getByTestId("node-tags-save").click();
        await expect(row.getByRole("button", { name: `编辑 ${name} 的标签`, exact: true })).toBeVisible();
      }
      expect((await getTags(base, admin)).assignments.filter(entry => entry.tags.length > 0)).toHaveLength(3);
      // Editing the catalog after assignment must not delete referenced rows.
      await page.getByTestId("tag-new-name").fill("unused"); await page.getByTestId("tag-add").click();
      await page.getByTestId("tag-catalog-save").click();
      await expect.poll(async () => (await getTags(base, admin)).tags.length).toBe(3);
      expect((await getTags(base, admin)).assignments.filter(entry => entry.tags.length > 0)).toHaveLength(3);
    });

    await test.step("one visual bidirectional rule saves endpoint selectors, not paths", async () => {
      await page.goto(ui.at("/access"));
      await page.getByTestId("access-add-rule").click();
      const builder = page.getByTestId("access-rule-builder");
      await builder.getByTestId("access-rule-source").getByRole("checkbox", { name: /^client/ }).check();
      await builder.getByTestId("access-rule-destination").getByRole("checkbox", { name: /^server/ }).check();
      const direction = builder.getByTestId("access-rule-bidirectional");
      await expect(direction).toHaveAttribute("aria-checked", "true");
      await direction.click(); await expect(direction).toHaveAttribute("aria-checked", "false");
      await direction.click(); await expect(direction).toHaveAttribute("aria-checked", "true");
      await builder.getByTestId("access-rule-id").fill("tag-pair");
      await builder.getByTestId("access-rule-apply").click();
      await expect(builder).not.toBeVisible();
      expect((await getMeshAccess(base, admin)).policy.rules).toHaveLength(0);
      const mutation = page.waitForRequest(request => request.method() === "POST" && new URL(request.url()).pathname === ui.path("/access"));
      await page.getByTestId("access-save").click(); const request = await mutation;
      await expect(page.getByTestId("access-save-status")).toContainText("已保存");
      const saved = await getMeshAccess(base, admin);
      expect(saved.policy.rules).toEqual([{ id: "tag-pair", source: ["tag:client"], destination: ["tag:server"], action: "allow", bidirectional: true }]);
      expect(saved.source_jsonc).not.toBeNull();
      await expect(page.getByTestId("access-rule-card")).toHaveCount(1);
      const flow = { source_node_id: sha.id, destination_node_id: nyc.id, destination_ip: nyc.mesh_ipv4, domain: null, destination_country: null };
      const preview = await previewFlow(base, admin, { flow });
      expect(preview.authorization.action).toBe("allow"); expect(preview.authorization.matched_direction).toBe("reverse");
      expect(preview.routing.path).toEqual([sha.id, nyc.id]);
      const headers = Object.fromEntries(Object.entries(await request.allHeaders()).filter(([name]) => ["content-type", "next-action", "next-router-state-tree"].includes(name.toLowerCase())));
      const body = request.postData(); if (!body) throw new Error("Missing access action body");
      const response = await anonymous.request.post(request.url(), { headers: { ...headers, origin: webUrl }, data: body, maxRedirects: 0 });
      expect(response.headers()["x-action-redirect"] ?? response.headers().location ?? "").toContain("/login");
      expect(await getMeshAccess(base, admin)).toEqual(saved);
    });

    await test.step("schema errors and comments survive both views and navigation, then persist", async () => {
      const saved = await getMeshAccess(base, admin);
      // CodeMirror virtualizes its DOM: fill known complete JSON, never use innerText as the document.
      const source = `// endpoint policy comment\n${JSON.stringify(saved.policy, null, 2)}\n`;
      await page.getByRole("button", { name: "JSONC", exact: true }).click();
      const editor = page.getByRole("textbox", { name: "访问授权 JSONC", exact: true });
      await editor.fill(source.replace('"default_action": "reject"', '"default_action": "invalid"'));
      await expect(page.getByRole("list", { name: "JSONC 问题" })).toBeVisible();
      await page.getByRole("button", { name: "规则", exact: true }).click();
      await expect(page.getByTestId("access-text-invalid")).toBeVisible();
      await expect(page.getByTestId("access-save")).toBeDisabled();
      await page.getByRole("link", { name: "诊断", exact: true }).first().click();
      await expect(page).toHaveURL(ui.at("/diagnostics")); await page.goBack();
      await expect(page).toHaveURL(ui.at("/access"));
      await expect(page.getByTestId("access-text-invalid")).toBeVisible();
      await page.getByRole("button", { name: "JSONC", exact: true }).click();
      await expect(editor).toContainText("endpoint policy comment");
      await editor.fill(source); await expect(page.getByRole("list", { name: "JSONC 问题" })).toHaveCount(0);
      await page.getByTestId("access-save").click();
      await expect.poll(async () => (await getMeshAccess(base, admin)).source_jsonc).toBe(source);
      await page.reload(); await page.getByRole("button", { name: "JSONC", exact: true }).click();
      await expect(editor).toContainText("endpoint policy comment");
      const baseRevision = await page.getByTestId("access-base-revision").innerText();
      await editor.fill(`// competing draft\n${source}`);
      const latest = await getMeshAccess(base, admin);
      await setNodeCountry(base, admin, { node_id: nyc.id, country: "US", expected_revision: latest.revision });
      await page.getByTestId("access-save").click();
      await expect(page.getByTestId("access-save-status")).toContainText("冲突");
      await expect(page.getByTestId("access-base-revision")).toHaveText(baseRevision);
      await expect(editor).toContainText("competing draft");
      expect((await getMeshAccess(base, admin)).source_jsonc).toBe(source);
    });

    await test.step("routing JSONC is persisted with its own graphical draft", async () => {
      await page.goto(ui.at("/routing"));
      await page.getByRole("button", { name: "JSONC", exact: true }).click();
      const editor = page.getByRole("textbox", { name: "路由策略 JSONC", exact: true });
      const source = `// routing source comment\n${JSON.stringify((await getPolicy(base, admin)).policy, null, 2)}\n`;
      await editor.fill(source); await page.getByRole("button", { name: "图形", exact: true }).click();
      await page.getByRole("button", { name: "保存策略", exact: true }).click();
      await expect.poll(async () => (await getPolicy(base, admin)).source_jsonc).toBe(source);
      await page.reload(); await page.getByRole("button", { name: "JSONC", exact: true }).click();
      await expect(editor).toContainText("routing source comment");
    });

    await test.step("revocation removes tag membership and mobile rule cards remain legible", async () => {
      await revokeNode(base, admin, { node_id: sha.id });
      expect((await getTags(base, admin)).assignments.some(entry => entry.node_id === sha.id)).toBe(false);
      // The tag itself remains a valid, now empty subject group.
      expect((await getMeshAccess(base, admin)).valid).toBe(true);
      await page.goto(ui.at("/access"));
      await page.getByRole("button", { name: "规则", exact: true }).click();
      await page.setViewportSize({ width: 390, height: 844 });
      await expectNoOverflow(page);
      await page.screenshot({ path: test.info().outputPath("access-v2-mobile.png"), fullPage: true });
      expect((await page.content()).includes(admin)).toBe(false);
    });
    expect(errors).toEqual([]);
  } finally {
    await anonymous.close(); await web?.stop(); await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
