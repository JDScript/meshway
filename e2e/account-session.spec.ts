import { expect, test, type Page, type Response } from "@playwright/test";
import { setTimeout as delay } from "node:timers/promises";
import * as sdk from "./network-client.ts";
import { accountStack, activeCredentialCookie, approve, credentialCookies, deviceRequest, invitedAccount, replayHeaders, signIn, testPassword } from "./accounts.ts";
import { expectNoOverflow } from "./harness.ts";

/** Every HTML / Flight body the browser received, to prove no secret crossed to the client. */
function recordBodies(page: Page) {
  const bodies: Promise<string>[] = [];
  page.on("response", (response: Response) => {
    const type = response.headers()["content-type"] ?? "";
    // Bodies of pages navigated away from mid-stream may never complete.
    if (/text\/html|text\/x-component/.test(type)) bodies.push(Promise.race([response.text().catch(() => ""), delay(3000, "")]));
  });
  return async () => (await Promise.all(bodies)).join("\n");
}

async function sessionToken(page: Page): Promise<string> {
  const cookie = await activeCredentialCookie(page.context());
  if (!cookie) throw new Error("Missing console session cookie");
  expect(cookie.httpOnly).toBe(true);
  expect(cookie.sameSite).toBe("Strict");
  const token = cookie.value.split(".").at(-1) ?? "";
  expect(token).toMatch(/^mws_[0-9a-f]{64}$/);
  return token;
}

test("accounts: email login, invitation, Member read-only without logout, network isolation, session end before exchange, CSRF and draft isolation", async ({ page, browser }) => {
  // Several real Argon2id sign-ins and registrations.
  test.setTimeout(240_000);
  const stack = await accountStack("account-e2e");
  const { base, webUrl, admin, networkId } = stack;
  const anonymous = await browser.newContext();
  const memberContext = await browser.newContext();
  const bodies = recordBodies(page);
  const secrets: string[] = [admin];
  const logOnlySecrets: string[] = [];
  try {
    // Fixture Owner (a real account made through the bootstrap claim API) and a second network it is not in.
    const ownerEmail = "owner@example.test"; const ownerPassword = testPassword("owner"); secrets.push(ownerPassword);
    await sdk.claimFirstOwner(base, admin, networkId, { kind: "new_account", email: ownerEmail, password: ownerPassword });
    const other = await sdk.createNetwork(base, admin, { name: "Other network" });

    await test.step("email login keeps the controller session in an HttpOnly cookie and ignores arbitrary return targets", async () => {
      await page.goto(`${webUrl}/login?returnTo=${encodeURIComponent("https://evil.test/")}`);
      await page.getByLabel("邮箱", { exact: true }).fill(ownerEmail);
      await page.getByLabel("密码", { exact: true }).fill("wrong-password-123");
      await page.getByRole("button", { name: "使用邮箱登录", exact: true }).click();
      await expect(page.getByRole("alert").filter({ hasText: /./ })).toContainText("邮箱或密码不正确");
      // The email survives a failed attempt; the password does not.
      await expect(page.getByLabel("邮箱", { exact: true })).toHaveValue(ownerEmail);
      await expect(page.getByLabel("密码", { exact: true })).toHaveValue("");
      expect((await page.context().cookies()).filter(cookie => cookie.httpOnly)).toHaveLength(0);
      await page.setViewportSize({ width: 390, height: 844 });
      await expectNoOverflow(page);
      await page.setViewportSize({ width: 1440, height: 1000 });
      await signIn(page, ownerEmail, ownerPassword);
      await expect(page).toHaveURL(`${webUrl}/${networkId}`);
      const token = await sessionToken(page); secrets.push(token);
      expect(await page.evaluate(() => document.cookie)).not.toContain("mws_");
      const cookie = await activeCredentialCookie(page.context());
      // The controller's absolute expiry (≤ 7 days), not a Web-extended one.
      expect(cookie?.expires).toBeLessThanOrEqual(Date.now() / 1000 + 7 * 86400 + 5);
      await page.goto(`${webUrl}/${networkId}/machines`);
      await page.getByRole("button", { name: "账户菜单", exact: true }).click();
      await expect(page.getByRole("menu")).toContainText(ownerEmail);
      await expect(page.getByRole("menu")).toContainText("本网络所有者");
      await page.keyboard.press("Escape");
      // Users only see their own networks; another network is a 404, not a fallback.
      expect((await page.goto(`${webUrl}/${other.network_id}/machines`))?.status()).toBe(404);
    });

    let member = { email: "member@example.test", password: "" };
    await test.step("an Owner issues an invitation link; the invitee registers through it", async () => {
      await page.goto(`${webUrl}/invitations/new?network=${networkId}`);
      await page.getByLabel("受邀邮箱", { exact: true }).fill(member.email);
      await page.getByRole("button", { name: "创建邀请链接", exact: true }).click();
      const link = await page.getByRole("status").getByRole("textbox").inputValue();
      expect(new URL(link).pathname).toBe("/invite");
      expect(new URL(link).search).toBe("");
      // Shown once to the Owner by design, so only checked against server logs.
      logOnlySecrets.push(new URL(link).hash.slice("#token=".length));
      member.password = testPassword("member"); secrets.push(member.password);
      const invitee = await memberContext.newPage();
      await invitee.goto(link);
      await expect(invitee).toHaveURL(`${webUrl}/invite`);
      await invitee.getByLabel("邮箱", { exact: true }).fill(member.email);
      await invitee.getByLabel("设置密码", { exact: true }).fill(member.password);
      await invitee.getByLabel("确认密码", { exact: true }).fill(member.password);
      await invitee.getByRole("button", { name: "注册并加入", exact: true }).click();
      await expect(invitee).toHaveURL(`${webUrl}/${networkId}`);
      await invitee.close();
    });

    await test.step("a Member reads, gets a 403 for writes without being signed out, and cannot reach other networks", async () => {
      const memberPage = await memberContext.newPage();
      await memberPage.goto(`${webUrl}/${networkId}/machines/add`);
      await memberPage.getByRole("button", { name: "生成接入令牌", exact: true }).click();
      await expect(memberPage.getByRole("alert").filter({ hasText: /./ })).toContainText("当前账户没有此操作的权限");
      await expect(memberPage).toHaveURL(`${webUrl}/${networkId}/machines/add`);
      await memberPage.goto(`${webUrl}/${networkId}/machines`);
      await expect(memberPage).toHaveURL(`${webUrl}/${networkId}/machines`);
      await memberPage.getByRole("button", { name: "账户菜单", exact: true }).click();
      await expect(memberPage.getByRole("menu")).toContainText("本网络成员");
      await expect(memberPage.getByRole("menuitem", { name: "邀请成员" })).toHaveCount(0);
      await memberPage.keyboard.press("Escape");
      expect((await memberPage.goto(`${webUrl}/${other.network_id}`))?.status()).toBe(404);
      // A Member approves their own device, only into their own network.
      const device = deviceRequest("Member phone");
      await memberPage.goto(device.url(webUrl));
      const options = await memberPage.getByLabel("接入网络", { exact: true }).locator("option").allTextContents();
      expect(options).toHaveLength(1);
      expect(options[0]).toContain("成员");
      const { submitted, callback } = await approve(memberPage);
      const code = callback?.searchParams.get("code"); if (!code) throw new Error("Missing member code");
      // Replaying the same approval with another network ID is refused by the controller.
      const forged = (submitted.postData() ?? "").replaceAll(networkId, other.network_id);
      const crossNetwork = await memberPage.request.post(device.url(webUrl), { headers: { ...(await replayHeaders(submitted)), origin: webUrl }, data: forged, maxRedirects: 0 });
      expect(crossNetwork.headers()["x-action-redirect"] ?? "").not.toContain("meshway://");
      const node = await sdk.exchangeDevice(webUrl, code, device.binding);
      const memberSession = await sdk.loginAccount(base, { email: member.email, password: member.password });
      const membership = (await sdk.getAccountSession(base, memberSession.session_token)).memberships[0];
      const owned = (await sdk.listNodeOwnership(base, memberSession.session_token, networkId)).nodes.find(item => item.node_id === node.node.id);
      expect(owned?.membership_id).toBe(membership?.membership_id);
      await sdk.logoutAccount(base, memberSession.session_token);
      await memberPage.setViewportSize({ width: 390, height: 844 });
      await memberPage.goto(device.url(webUrl));
      await expectNoOverflow(memberPage);
      await memberPage.close();
    });

    await test.step("logout revokes the controller session: a code approved before it cannot be exchanged", async () => {
      const memberPage = await memberContext.newPage();
      const device = deviceRequest("Logout race");
      await memberPage.goto(device.url(webUrl));
      const { callback } = await approve(memberPage);
      const code = callback?.searchParams.get("code"); if (!code) throw new Error("Missing code");
      await memberPage.goto(`${webUrl}/${networkId}/machines`);
      const revoked = await sessionToken(memberPage);
      await memberPage.getByRole("button", { name: "账户菜单", exact: true }).click();
      await memberPage.getByRole("menuitem", { name: "退出登录", exact: true }).click();
      await expect(memberPage).toHaveURL(`${webUrl}/login`);
      await expect(sdk.getAccountSession(base, revoked)).rejects.toMatchObject({ status: 401 });
      await expect(sdk.exchangeDevice(webUrl, code, device.binding)).rejects.toThrow();
      expect(await credentialCookies(memberContext)).toHaveLength(0);
      await memberPage.close();
    });

    await test.step("a password reset ends every session: pending codes fail and the browser is sent to login", async () => {
      const memberPage = await memberContext.newPage();
      await memberPage.goto(`${webUrl}/login`);
      await signIn(memberPage, member.email, member.password);
      const device = deviceRequest("Reset race");
      await memberPage.goto(device.url(webUrl));
      const { callback } = await approve(memberPage);
      const code = callback?.searchParams.get("code"); if (!code) throw new Error("Missing code");
      const userId = (await sdk.getAccountSession(base, await sessionToken(memberPage))).user.user_id;
      const reset = await sdk.createAccountPasswordReset(base, admin, { user_id: userId, expires_in_seconds: 600 });
      const next = testPassword("reset"); secrets.push(next); logOnlySecrets.push(reset.reset_token);
      await sdk.redeemAccountPasswordReset(base, { reset_token: reset.reset_token, new_password: next });
      await expect(sdk.exchangeDevice(webUrl, code, device.binding)).rejects.toThrow();
      await memberPage.goto(`${webUrl}/${networkId}/machines`);
      await expect(memberPage).toHaveURL(/\/login\?reason=expired/);
      // An approval attempt with the dead session cannot produce a code either.
      await memberPage.goto(device.url(webUrl));
      await expect(memberPage).toHaveURL(/\/login\?returnTo=.*reason=expired|\/login\?returnTo=/);
      member = { ...member, password: next };
      await memberPage.close();
    });

    await test.step("the bootstrap console still manages; it cannot approve devices, and cross-origin or anonymous approvals fail", async () => {
      const adminContext = await browser.newContext();
      const adminPage = await adminContext.newPage();
      await adminPage.goto(`${webUrl}/login`);
      await adminPage.getByLabel("管理员令牌", { exact: true }).fill(admin);
      await adminPage.getByRole("button", { name: "登录", exact: true }).click();
      await expect(adminPage).toHaveURL(`${webUrl}/${networkId}`);
      await adminPage.goto(`${webUrl}/${networkId}/machines`);
      await adminPage.getByRole("button", { name: "账户菜单", exact: true }).click();
      await expect(adminPage.getByRole("menu")).toContainText("实例管理员");
      await adminPage.keyboard.press("Escape");
      await adminPage.goto(`${webUrl}/${networkId}/machines/add`);
      await adminPage.getByRole("button", { name: "生成接入令牌", exact: true }).click();
      await expect(adminPage.getByTestId("enrollment-token")).toBeVisible();
      // A user session pasted as the administrator token is refused.
      const userToken = await sessionToken(page);
      const probe = await anonymous.newPage();
      await probe.goto(`${webUrl}/login`);
      await probe.getByLabel("管理员令牌", { exact: true }).fill(userToken);
      await probe.getByRole("button", { name: "登录", exact: true }).click();
      await expect(probe.getByRole("alert").filter({ hasText: /./ })).toContainText("不接受这个管理员令牌");
      await probe.close();

      const device = deviceRequest("Bootstrap attempt");
      await adminPage.goto(device.url(webUrl));
      await expect(adminPage.getByRole("heading", { name: "需要账户登录" })).toBeVisible();
      // Owner's own approval request, replayed cross-origin with the Owner's cookie, and anonymously.
      await page.goto(device.url(webUrl));
      const before = (await sdk.listNodes(base, admin)).nodes.length;
      const request = page.waitForRequest(r => r.method() === "POST" && new URL(r.url()).pathname === "/authorize-device");
      await page.route("**/authorize-device*", route => route.request().method() === "POST" ? route.abort() : route.continue());
      await page.getByRole("button", { name: "批准接入", exact: true }).click();
      const submitted = await request;
      await page.unroute("**/authorize-device*");
      const headers = await replayHeaders(submitted);
      const crossOrigin = await page.request.post(device.url(webUrl), { headers: { ...headers, origin: "http://evil.test" }, data: submitted.postData() ?? "", maxRedirects: 0 });
      expect(crossOrigin.headers()["x-action-redirect"] ?? crossOrigin.headers().location ?? "").not.toContain("meshway://");
      const anonymousReplay = await anonymous.request.post(device.url(webUrl), { headers: { ...headers, origin: webUrl }, data: submitted.postData() ?? "", maxRedirects: 0 });
      expect(anonymousReplay.headers()["x-action-redirect"] ?? anonymousReplay.headers().location).toContain("/login");
      const bootstrapReplay = await adminPage.request.post(device.url(webUrl), { headers: { ...headers, origin: webUrl }, data: submitted.postData() ?? "", maxRedirects: 0 });
      expect(bootstrapReplay.headers()["x-action-redirect"] ?? "").not.toContain("meshway://");
      // The Owner's rendered form never runs with the administrator cookie (sign-in fence).
      expect(await bootstrapReplay.text()).toContain("这个页面属于之前的登录");
      expect((await sdk.listNodes(base, admin)).nodes.length).toBe(before);

      // First Owner of the other network: an existing account proven by its own password.
      await adminPage.goto(`${webUrl}/onboarding`);
      await adminPage.setViewportSize({ width: 390, height: 844 });
      await expectNoOverflow(adminPage);
      const section = adminPage.getByRole("region", { name: "Other network" });
      await expect(adminPage.getByRole("region", { name: "Meshway" })).toContainText(`已有所有者：${ownerEmail}`);
      await section.getByLabel("账户", { exact: true }).selectOption("existing_account");
      await section.getByLabel("邮箱", { exact: true }).fill(ownerEmail);
      await section.getByLabel("密码", { exact: true }).fill("not-the-password-1");
      await section.getByRole("button", { name: "认领为所有者", exact: true }).click();
      await expect(section.getByRole("alert").filter({ hasText: /./ })).toContainText("邮箱或密码不正确");
      await section.getByLabel("密码", { exact: true }).fill(ownerPassword);
      await section.getByRole("button", { name: "认领为所有者", exact: true }).click();
      await expect(section.getByRole("status")).toContainText(`${ownerEmail} 已成为「Other network」的所有者`);
      const ownerSession = await sdk.loginAccount(base, { email: ownerEmail, password: ownerPassword });
      const roles = (await sdk.getAccountSession(base, ownerSession.session_token)).memberships.map(item => [item.network_id, item.role]);
      expect(roles).toContainEqual([other.network_id, "owner"]);
      await sdk.logoutAccount(base, ownerSession.session_token);
      await adminContext.close();
    });

    await test.step("switching accounts in another tab drops every draft of the old sign-in", async () => {
      await page.goto(`${webUrl}/${networkId}/access`);
      await page.getByRole("button", { name: "JSONC", exact: true }).click();
      await page.getByRole("textbox", { name: "访问授权 JSONC", exact: true }).fill("// owner draft\n{ invalid");
      await page.evaluate(() => Reflect.set(window, "meshwayDraftMarker", true));
      const second = await page.context().newPage();
      await second.goto(`${webUrl}/${networkId}/machines`);
      await second.getByRole("button", { name: "账户菜单", exact: true }).click();
      await second.getByRole("menuitem", { name: "退出登录", exact: true }).click();
      await expect(second).toHaveURL(`${webUrl}/login`);
      await signIn(second, member.email, member.password);
      await page.bringToFront();
      // The old tab notices the sign-out by itself (no navigation by the test) and
      // drops the draft with a reload, which lands on login once the cookie is gone.
      await expect.poll(() => page.evaluate(() => Reflect.get(window, "meshwayDraftMarker")), { timeout: 15_000 }).toBeUndefined();
      await expect(page).toHaveURL(/\/login|\/access$/);
      await page.goto(`${webUrl}/${networkId}/access`);
      await page.getByRole("button", { name: "JSONC", exact: true }).click();
      await expect(page.getByRole("textbox", { name: "访问授权 JSONC", exact: true })).not.toContainText("owner draft");
      await page.getByRole("button", { name: "账户菜单", exact: true }).click();
      await expect(page.getByRole("menu")).toContainText(member.email);
      await page.keyboard.press("Escape");
      await second.close();
    });

    await test.step("no password, session, invitation or administrator secret reaches HTML, Flight or server logs", async () => {
      const received = await bodies();
      // The recorder really read rendered pages.
      expect(received).toContain(ownerEmail);
      const logs = `${stack.web.logs()}\n${stack.controller.logs()}`;
      for (const secret of secrets.filter(Boolean)) {
        expect(received.includes(secret), "secret in a browser response").toBe(false);
        expect(logs.includes(secret), "secret in a server log").toBe(false);
      }
      for (const secret of logOnlySecrets) expect(logs.includes(secret), "invitation in a server log").toBe(false);
    });
  } finally {
    await memberContext.close(); await anonymous.close(); await stack.stop();
  }
});
