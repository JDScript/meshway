// Account fixtures: a real temporary controller and Web server, real accounts
// created through the generated API or the Web UI. Nothing injects a session.
import { expect, type Page } from "@playwright/test";
import { createHash, randomBytes } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import * as sdk from "../packages/control-client/src/index.ts";
import { executable, root, service, unusedPort } from "./harness.ts";

export async function accountStack(prefix: string) {
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), `meshway-${prefix}-`));
  const controllerPort = await unusedPort(); const webPort = await unusedPort();
  const base = `http://127.0.0.1:${controllerPort}`; const webUrl = `http://127.0.0.1:${webPort}`;
  const controller = service(executable("meshway-controller"), ["serve", "--data-dir", join(temporary, "controller"), "--bind", `127.0.0.1:${controllerPort}`]);
  let web: ReturnType<typeof service> | undefined;
  try {
    await controller.ready(`${base}/api/v1/overview`, 401);
    const admin = (await readFile(join(temporary, "controller/admin.token"), "utf8")).trim();
    web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
      ...process.env, MESHWAY_CONTROLLER_URL: base, MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    const network = (await sdk.listNetworks(base, admin)).networks[0];
    if (!network) throw new Error("Fixture controller has no network");
    const started = web;
    return {
      base, webUrl, admin, networkId: network.network_id, web: started, controller,
      async stop() { await started.stop(); await controller.stop(); await rm(temporary, { recursive: true, force: true }); },
    };
  } catch (error) {
    await web?.stop(); await controller.stop(); await rm(temporary, { recursive: true, force: true });
    throw error;
  }
}

/** Fixture-only test passwords; never a real account. */
export const testPassword = (label: string) => `e2e-${label}-${randomBytes(8).toString("hex")}`;

/** A real invited account in `networkId`, issued by `issuer` (Owner session or bootstrap). */
export async function invitedAccount(base: string, issuer: string, networkId: string, email: string, role: sdk.AccountRole) {
  const password = testPassword(role);
  const invitation = await sdk.createAccountInvitation(base, issuer, networkId, { email, role, expires_in_seconds: 3600 });
  const membership = await sdk.registerInvitedAccount(base, { email, password, invitation_token: invitation.invitation_token });
  return { email, password, membership };
}

export async function signIn(page: Page, email: string, password: string) {
  await page.getByLabel("邮箱", { exact: true }).fill(email);
  await page.getByLabel("密码", { exact: true }).fill(password);
  await page.getByRole("button", { name: "使用邮箱登录", exact: true }).click();
  await expect(page).not.toHaveURL(/\/login(?:\?|$)/);
}

export function deviceRequest(name: string) {
  const verifier = randomBytes(32).toString("base64url");
  const params = { redirect_uri: "meshway://enroll/callback", state: `state-${randomBytes(4).toString("hex")}`, code_challenge: createHash("sha256").update(verifier).digest("base64url"), name, agent_version: "e2e" };
  const credential = randomBytes(32).toString("hex"); // persisted by a real App before enrollment
  return {
    params, verifier, credential,
    url: (webUrl: string) => `${webUrl}/authorize-device?${new URLSearchParams(params)}`,
    binding: { code_verifier: verifier, redirect_uri: params.redirect_uri, node_credential: credential, name: params.name, agent_version: params.agent_version },
  };
}

const actionResponse = (page: Page, path: string) =>
  page.waitForResponse(r => r.request().method() === "POST" && new URL(r.url()).pathname === path);

/** Clicks 批准接入 on an open authorize page; returns the App callback, the action request and its result. */
export async function approve(page: Page, network?: string) {
  if (network) await page.getByLabel("接入网络", { exact: true }).selectOption(network);
  const request = page.waitForRequest(r => r.method() === "POST" && new URL(r.url()).pathname === "/authorize-device");
  const response = actionResponse(page, "/authorize-device");
  await page.getByRole("button", { name: "批准接入", exact: true }).click();
  const [submitted, result] = await Promise.all([request, response]);
  const location = (result.headers()["x-action-redirect"] ?? result.headers().location ?? "").split(";")[0] ?? "";
  return { submitted, callback: location ? new URL(location) : null };
}

export async function replayHeaders(request: { allHeaders(): Promise<Record<string, string>> }) {
  return Object.fromEntries(Object.entries(await request.allHeaders()).filter(([key]) => ["content-type", "next-action", "next-router-state-tree"].includes(key)));
}
