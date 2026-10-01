import { expect, test } from "@playwright/test";
import { createHash, randomBytes } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { exchangeDevice, listNodes, listNetworks, authorizeDevice } from "./network-client.ts";
import { heartbeatAgentOperation as heartbeatAgent, getAgentConfigurationOperation, getAgentDnsOperation } from "../packages/control-client/src/generated/operations.gen.ts";
import { executable, root, service, unusedPort } from "./harness.ts";

test("browser SSO approval redirects a PKCE code, exchange creates a real unified node, deny creates none", async ({ page, browser }) => {
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-device-e2e-"));
  const controllerPort = await unusedPort(); const webPort = await unusedPort();
  const base = `http://127.0.0.1:${controllerPort}`; const webUrl = `http://127.0.0.1:${webPort}`;
  const controller = service(executable("meshway-controller"), ["serve", "--data-dir", join(temporary, "controller"), "--bind", `127.0.0.1:${controllerPort}`]);
  let web: ReturnType<typeof service> | undefined;
  const anonymous = await browser.newContext();
  try {
    await controller.ready(`${base}/api/v1/overview`, 401);
    const admin = (await readFile(join(temporary, "controller/admin.token"), "utf8")).trim();
    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: base, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    const verifier = randomBytes(32).toString("base64url");
    const credential = randomBytes(32).toString("hex"); // persisted by real App before enrollment
    const params = { redirect_uri: "meshway://enroll/callback", state: "opaque+state", code_challenge: createHash("sha256").update(verifier).digest("base64url"), name: "SSO Mac", agent_version: "e2e" };
    const authorize = `${webUrl}/authorize-device?${new URLSearchParams(params)}`;
    await page.goto(authorize);
    await expect(page).toHaveURL(/\/login\?returnTo=/);
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page.getByRole("heading", { name: "授权设备接入" })).toBeVisible();
    await expect(page.getByText("SSO Mac", { exact: true })).toBeVisible();
    const mutation = page.waitForRequest(r => r.method() === "POST" && new URL(r.url()).pathname === "/authorize-device");
    const response = page.waitForResponse(r => r.request().method() === "POST" && new URL(r.url()).pathname === "/authorize-device");
    await page.getByRole("button", { name: "批准接入", exact: true }).click();
    const submitted = await mutation;
    const result = await response;
    const location = (result.headers()["x-action-redirect"] ?? result.headers().location ?? "").split(";")[0];
    const callback = new URL(location ?? "");
    expect(`${callback.protocol}//${callback.host}${callback.pathname}`).toBe(params.redirect_uri);
    expect(callback.searchParams.get("state")).toBe(params.state);
    const code = callback.searchParams.get("code"); if (!code) throw new Error("Missing exchange code");
    const body = submitted.postData(); if (!body) throw new Error("Missing action body");
    const headers = Object.fromEntries(Object.entries(await submitted.allHeaders()).filter(([key]) => ["content-type", "next-action", "next-router-state-tree"].includes(key)));
    const blocked = await anonymous.request.post(authorize, { headers: { ...headers, origin: webUrl }, data: body, maxRedirects: 0 });
    expect(blocked.headers()["x-action-redirect"] ?? blocked.headers().location).toContain("/login");
    expect((await listNodes(base, admin)).nodes).toHaveLength(0);
    const binding = { code_verifier: verifier, redirect_uri: params.redirect_uri, node_credential: credential, name: params.name, agent_version: params.agent_version };
    const node = await exchangeDevice(webUrl, code, binding);
    expect(node.node.name).toBe("SSO Mac"); expect(node.node.revoked_at).toBeNull();
    expect((await exchangeDevice(webUrl, code, binding)).node.id).toBe(node.node.id);
    expect((await heartbeatAgent(webUrl, credential, { agent_version: params.agent_version })).node.id).toBe(node.node.id);
    const configuration = await getAgentConfigurationOperation(webUrl, credential);
    expect(configuration.payload.node.id).toBe(node.node.id);
    expect(configuration.payload.peers).toHaveLength(0);
    expect((await getAgentDnsOperation(webUrl, credential)).payload.node_id).toBe(node.node.id);
    // A second real enrollment must be visible through the node-scoped GET proxy.
    const peerCredential = randomBytes(32).toString("hex");
    const networkId = (await listNetworks(base, admin)).networks[0]!.network_id;
    const peerCode = (await authorizeDevice(base, admin, networkId, {
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
    for (const path of ["nodes", "overview", "device-authorizations", "nodes/revoke", "enrollment-tokens"]) {
      const response = await anonymous.request.post(`${webUrl}/api/v1/${path}`, { headers: { authorization: `Bearer ${admin}` }, data: {} });
      expect(response.status()).toBe(404);
      const read = await anonymous.request.get(`${webUrl}/api/v1/${path}`, { headers: { authorization: `Bearer ${admin}` } });
      expect(read.status()).toBe(404);
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
    await anonymous.close(); await web?.stop(); await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
