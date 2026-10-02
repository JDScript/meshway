import { expect, test } from "@playwright/test";
import { randomBytes } from "node:crypto";
import { exchangeDevice, listNodes, authorizeDevice, getAccountSession, listNodeOwnership, loginAccount } from "./network-client.ts";
import { heartbeatAgentOperation as heartbeatAgent, getAgentConfigurationOperation, getAgentDnsOperation } from "../packages/control-client/src/generated/operations.gen.ts";
import { accountStack, approve, deviceRequest, replayHeaders, signIn, testPassword } from "./accounts.ts";

test("browser SSO: first Owner from the bootstrap session, account approval redirects a PKCE code, exchange creates a user-owned node, deny creates none", async ({ page, browser }) => {
  const stack = await accountStack("device-e2e");
  const { base, webUrl, admin, networkId } = stack;
  const anonymous = await browser.newContext();
  try {
    const device = deviceRequest("SSO Mac");
    const { params, binding } = device;
    const authorize = device.url(webUrl);
    await page.goto(authorize);
    await expect(page).toHaveURL(/\/login\?returnTo=/);

    // The administrator token still signs in, but is not a user: no approval form.
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page.getByRole("heading", { name: "需要账户登录" })).toBeVisible();
    await expect(page.getByRole("button", { name: "批准接入", exact: true })).toHaveCount(0);
    await expect(authorizeDevice(base, admin, networkId, { redirect_uri: params.redirect_uri, code_challenge: params.code_challenge, name: "x", agent_version: "e2e" }))
      .rejects.toMatchObject({ status: expect.any(Number) });

    // First Owner: explicit, with the person's own email and password.
    const email = "owner@example.test"; const password = testPassword("owner");
    await page.getByRole("link", { name: "还没有账户？为网络认领第一个所有者" }).click();
    await expect(page.getByRole("heading", { name: "认领网络所有者" })).toBeVisible();
    await page.getByLabel("邮箱", { exact: true }).fill(email);
    await page.getByLabel("设置密码", { exact: true }).fill(password);
    await page.getByLabel("确认密码", { exact: true }).fill(password);
    await page.getByRole("button", { name: "认领为所有者", exact: true }).click();
    await expect(page.getByRole("status")).toContainText(`${email} 已成为`);
    expect(await page.content()).not.toContain(password);

    await page.goto(authorize);
    await page.getByRole("button", { name: "退出管理员会话并使用账户登录", exact: true }).click();
    await expect(page).toHaveURL(/\/login\?returnTo=/);
    await signIn(page, email, password);
    await expect(page.getByRole("heading", { name: "授权设备接入" })).toBeVisible();
    await expect(page.getByText("SSO Mac", { exact: true })).toBeVisible();
    await expect(page.getByText(email, { exact: true })).toBeVisible();
    // Viewing the page approves nothing.
    expect((await listNodes(base, admin)).nodes).toHaveLength(0);

    const { submitted, callback } = await approve(page);
    if (!callback) throw new Error("Missing App callback");
    expect(`${callback.protocol}//${callback.host}${callback.pathname}`).toBe(params.redirect_uri);
    expect(callback.searchParams.get("state")).toBe(params.state);
    const code = callback.searchParams.get("code"); if (!code) throw new Error("Missing exchange code");
    const body = submitted.postData(); if (!body) throw new Error("Missing action body");
    const headers = await replayHeaders(submitted);
    const blocked = await anonymous.request.post(authorize, { headers: { ...headers, origin: webUrl }, data: body, maxRedirects: 0 });
    expect(blocked.headers()["x-action-redirect"] ?? blocked.headers().location).toContain("/login");
    expect((await listNodes(base, admin)).nodes).toHaveLength(0);

    const node = await exchangeDevice(webUrl, code, binding);
    expect(node.node.name).toBe("SSO Mac"); expect(node.node.revoked_at).toBeNull();
    expect((await exchangeDevice(webUrl, code, binding)).node.id).toBe(node.node.id);
    // The node is owned by the approving Owner's membership, not unassigned.
    const owner = await loginAccount(base, { email, password });
    const membership = (await getAccountSession(base, owner.session_token)).memberships.find(item => item.network_id === networkId);
    expect(membership?.role).toBe("owner");
    const ownership = (await listNodeOwnership(base, owner.session_token, networkId)).nodes.find(item => item.node_id === node.node.id);
    expect(ownership?.membership_id).toBe(membership?.membership_id);

    const credential = binding.node_credential;
    expect((await heartbeatAgent(webUrl, credential, { agent_version: params.agent_version })).node.id).toBe(node.node.id);
    const configuration = await getAgentConfigurationOperation(webUrl, credential);
    expect(configuration.payload.node.id).toBe(node.node.id);
    expect(configuration.payload.peers).toHaveLength(0);
    expect((await getAgentDnsOperation(webUrl, credential)).payload.node_id).toBe(node.node.id);
    // A second real enrollment must be visible through the node-scoped GET proxy.
    const peerCredential = randomBytes(32).toString("hex");
    const peerCode = (await authorizeDevice(base, owner.session_token, networkId, {
      redirect_uri: params.redirect_uri, code_challenge: params.code_challenge, name: "Second device", agent_version: "e2e",
    })).code;
    await exchangeDevice(webUrl, peerCode, { ...binding, node_credential: peerCredential, name: "Second device" });
    expect((await getAgentConfigurationOperation(webUrl, credential)).payload.peers.map(peer => peer.name)).toEqual(["Second device"]);
    for (const path of ["agents/configuration", "agent/dns"]) {
      const rejected = await anonymous.request.get(`${webUrl}/api/v1/${path}`);
      expect(rejected.status()).toBe(401);
      expect(rejected.headers()["cache-control"]).toBe("no-store");
      expect((await anonymous.request.post(`${webUrl}/api/v1/${path}`, { data: {} })).status()).toBe(405);
    }
    const unauthenticated = await anonymous.request.post(`${webUrl}/api/v1/agents/heartbeat`, { data: { agent_version: "e2e" } });
    expect(unauthenticated.status()).toBe(401);
    expect(unauthenticated.headers()["www-authenticate"]).toBe("Bearer");
    expect(unauthenticated.headers()["cache-control"]).toBe("no-store");
    const directRejection = await anonymous.request.post(`${base}/api/v1/agents/heartbeat`, { data: { agent_version: "e2e" } });
    expect(await unauthenticated.text()).toBe(await directRejection.text());
    const wrongMedia = await anonymous.request.post(`${webUrl}/api/v1/agents/heartbeat`, {
      headers: { authorization: `Bearer ${credential}`, "content-type": "text/plain" }, data: "{}",
    });
    expect(wrongMedia.status()).toBe(415);
    for (const path of ["nodes", "overview", "device-authorizations", "nodes/revoke", "enrollment-tokens", "auth/login", "auth/session"]) {
      for (const bearer of [admin, owner.session_token]) {
        const response = await anonymous.request.post(`${webUrl}/api/v1/${path}`, { headers: { authorization: `Bearer ${bearer}` }, data: {} });
        expect(response.status()).toBe(404);
        const read = await anonymous.request.get(`${webUrl}/api/v1/${path}`, { headers: { authorization: `Bearer ${bearer}` } });
        expect(read.status()).toBe(404);
      }
    }
    expect((await listNodes(base, admin)).nodes).toHaveLength(2);
    await page.goto(authorize);
    const denial = page.waitForResponse(r => r.request().method() === "POST" && new URL(r.url()).pathname === "/authorize-device");
    await page.getByRole("button", { name: "拒绝", exact: true }).click();
    const denied = await denial;
    const deniedUrl = new URL((denied.headers()["x-action-redirect"] ?? denied.headers().location ?? "").split(";")[0] ?? "");
    expect(deniedUrl.searchParams.get("error")).toBe("access_denied"); expect(deniedUrl.searchParams.get("state")).toBe(params.state);
    expect((await listNodes(base, admin)).nodes).toHaveLength(2);
    await page.goto(`${webUrl}/authorize-device?${new URLSearchParams({ ...params, redirect_uri: "https://evil.test" })}`);
    await expect(page.getByRole("heading", { name: "授权请求无效" })).toBeVisible();
  } finally {
    await anonymous.close(); await stack.stop();
  }
});
