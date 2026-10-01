import { expect, test } from "@playwright/test";
import { createHash, randomBytes } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createEnrollmentToken, exchangeDevice, getMeshAccess, listNetworks, listNodes } from "../packages/control-client/src/index.ts";
import { enrollAgentOperation, getAgentConfigurationOperation, getAgentDnsOperation } from "../packages/control-client/src/generated/operations.gen.ts";
import { executable, root, service, unusedPort } from "./harness.ts";

test("network creation, empty inventory, scoped enrollment, browser approval and independent drafts", async ({ page, context, browser }) => {
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-multi-network-"));
  const base = `http://127.0.0.1:${await unusedPort()}`;
  const webPort = await unusedPort();
  const webUrl = `http://127.0.0.1:${webPort}`;
  const controller = service(executable("meshway-controller"), ["serve", "--data-dir", join(temporary, "controller"), "--bind", new URL(base).host]);
  let web: ReturnType<typeof service> | undefined;
  const anonymous = await browser.newContext();
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  try {
    await controller.ready(`${base}/api/v1/networks`, 401);
    const admin = (await readFile(join(temporary, "controller/admin.token"), "utf8")).trim();
    const initial = (await listNetworks(base, admin)).networks[0]!;
    const firstCredential = randomBytes(32).toString("hex");
    const token = await createEnrollmentToken(base, admin, initial.network_id, { expires_in_seconds: 600 });
    const first = (await enrollAgentOperation(base, token.token, { node_credential: firstCredential, name: "First member", agent_version: "e2e" })).node;
    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: base, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    await page.goto(`${webUrl}/nodes`);
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).not.toHaveURL(/\/login(?:\?|$)/);
    await page.goto(`${webUrl}/nodes`);
    await expect(page.getByText("First member", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "当前网络", exact: true }).click();
    await page.getByRole("menuitem", { name: "创建网络", exact: true }).click();
    await page.getByLabel("网络名称", { exact: true }).fill("Second network");
    const createRequest = page.waitForRequest(request => request.method() === "POST" && new URL(request.url()).pathname === "/nodes");
    await page.getByRole("button", { name: "创建网络", exact: true }).click();
    const submitted = await createRequest;
    await expect(page.getByRole("button", { name: "当前网络", exact: true })).toContainText("Second network");
    await expect(page.getByText("First member", { exact: true })).toHaveCount(0);
    const second = (await listNetworks(base, admin)).networks.find(network => network.name === "Second network")!;
    expect((await listNodes(base, admin, second.network_id)).nodes).toEqual([]);
    const requestBody = submitted.postData(); if (!requestBody) throw new Error("Missing network creation action body");
    const headers = Object.fromEntries(Object.entries(await submitted.allHeaders()).filter(([name]) => ["content-type", "next-action", "next-router-state-tree"].includes(name)));
    const replay = await anonymous.request.post(submitted.url(), { headers: { ...headers, origin: webUrl }, data: requestBody, maxRedirects: 0 });
    expect(replay.headers()["x-action-redirect"] ?? replay.headers().location).toContain("/login");
    expect((await listNetworks(base, admin)).networks).toHaveLength(2);

    await page.goto(`${webUrl}/enrollment`);
    await page.getByRole("button", { name: "创建注册令牌", exact: true }).click();
    const secondToken = await page.getByTestId("enrollment-token").innerText();
    const secondCredential = randomBytes(32).toString("hex");
    const member = (await enrollAgentOperation(base, secondToken.trim(), { node_credential: secondCredential, name: "Second member", agent_version: "e2e" })).node;
    expect(member.mesh_ipv4).toBe(first.mesh_ipv4);
    expect((await getAgentConfigurationOperation(base, firstCredential)).payload.peers).toEqual([]);
    expect((await getAgentConfigurationOperation(base, secondCredential)).payload.peers).toEqual([]);
    expect((await getAgentDnsOperation(base, secondCredential)).payload.network_id).toBe(second.network_id);

    const switchTo = async (name: string) => {
      await page.getByRole("button", { name: "当前网络", exact: true }).click();
      await page.getByRole("menuitem", { name, exact: true }).click();
      await expect(page.getByRole("button", { name: "当前网络", exact: true })).toContainText(name);
    };
    await page.goto(`${webUrl}/access`);
    const secondBase = await page.getByTestId("access-base-revision").innerText();
    await page.getByRole("button", { name: "JSONC", exact: true }).click();
    const invalid = "// second-network draft\n{ invalid";
    await page.getByRole("textbox", { name: "访问授权 JSONC", exact: true }).fill(invalid);
    await switchTo(initial.name);
    await page.getByRole("button", { name: "JSONC", exact: true }).click();
    await expect(page.getByRole("textbox", { name: "访问授权 JSONC", exact: true })).not.toContainText("second-network draft");
    // Keep a separate tab rendered in the first network; switching the cookie
    // elsewhere must not retarget its already-rendered save action.
    const oldTab = await context.newPage();
    await oldTab.goto(`${webUrl}/access`);
    await oldTab.getByRole("button", { name: "JSONC", exact: true }).click();
    const firstSource = `// first-network multi-tab save\n${JSON.stringify((await getMeshAccess(base, admin, initial.network_id)).policy)}`;
    await oldTab.getByRole("textbox", { name: "访问授权 JSONC", exact: true }).fill(firstSource);
    await switchTo(second.name);
    await expect(page.getByRole("textbox", { name: "访问授权 JSONC", exact: true }).locator(".cm-line")).toHaveText(invalid.split("\n"));
    await expect(page.getByTestId("access-base-revision")).toHaveText(secondBase);
    await oldTab.getByTestId("access-save").click();
    await expect(oldTab.getByTestId("access-save-status")).toContainText("已保存");
    expect((await getMeshAccess(base, admin, initial.network_id)).source_jsonc).toBe(firstSource);
    expect((await getMeshAccess(base, admin, second.network_id)).source_jsonc).toBeNull();
    await oldTab.close();

    const verifier = randomBytes(32).toString("base64url");
    const params = { redirect_uri: "meshway://enroll/callback", state: "multi-network", code_challenge: createHash("sha256").update(verifier).digest("base64url"), name: "Selected App", agent_version: "e2e" };
    await page.goto(`${webUrl}/authorize-device?${new URLSearchParams(params)}`);
    await page.getByLabel("接入网络").selectOption(second.network_id);
    const approval = page.waitForResponse(response => response.request().method() === "POST" && new URL(response.url()).pathname === "/authorize-device");
    await page.getByRole("button", { name: "批准接入", exact: true }).click();
    const result = await approval;
    const callback = new URL((result.headers()["x-action-redirect"] ?? result.headers().location ?? "").split(";")[0]!);
    const code = callback.searchParams.get("code"); if (!code) throw new Error("Missing exchange code");
    await exchangeDevice(webUrl, code, { code_verifier: verifier, redirect_uri: params.redirect_uri, node_credential: randomBytes(32).toString("hex"), name: params.name, agent_version: params.agent_version });
    expect((await listNodes(base, admin, initial.network_id)).nodes.map(node => node.name)).toEqual(["First member"]);
    expect((await listNodes(base, admin, second.network_id)).nodes.map(node => node.name).sort()).toEqual(["Second member", "Selected App"]);
    expect(errors).toEqual([]);
  } finally {
    await anonymous.close(); await web?.stop(); await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
