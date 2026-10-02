import { expect, test, type BrowserContext, type Page, type Route } from "@playwright/test";
import * as sdk from "./network-client.ts";
import { accountStack, activeCredentialCookie, credentialCookies, invitedAccount, signIn, testPassword } from "./accounts.ts";

// The response that actually carries a cookie deletion is held after the
// server produced it, and delivered to the browser only after another tab
// really signed in as B. The request is forwarded by Node's fetch (its own,
// cookie-less client), so Playwright never applies the Set-Cookie early.

const deletesConsoleCookie = (setCookies: string[]) =>
  setCookies.some(line => /^meshway_console_[^=]*=/.test(line) && (/max-age=0(?:;|$)/i.test(line) || /expires=thu, 01 jan 1970/i.test(line)));

/** Holds the first server response on `page` that deletes a console cookie; every other request passes through. */
async function holdFinalDeletion(page: Page) {
  let held: { route: Route; status: number; headers: Record<string, string>; body: Buffer } | undefined;
  const handler = async (route: Route) => {
    const request = route.request();
    if (request.method() !== "POST" || held) return route.fallback();
    const headers = await request.allHeaders();
    const response = await fetch(request.url(), {
      method: "POST",
      headers: Object.fromEntries(Object.entries(headers).filter(([name]) => !name.startsWith(":") && name !== "content-length")),
      body: (() => { const data = request.postDataBuffer(); return data ? new Uint8Array(data) : undefined; })(),
      redirect: "manual",
    });
    const setCookies = response.headers.getSetCookie();
    const forwarded: Record<string, string> = {};
    response.headers.forEach((value, name) => {
      if (!["set-cookie", "content-encoding", "content-length", "transfer-encoding"].includes(name)) forwarded[name] = value;
    });
    if (setCookies.length > 0) forwarded["set-cookie"] = setCookies.join("\n");
    const body = Buffer.from(await response.arrayBuffer());
    if (deletesConsoleCookie(setCookies)) {
      held = { route, status: response.status, headers: forwarded, body };
      return;
    }
    await route.fulfill({ status: response.status, headers: forwarded, body });
  };
  await page.route("**/*", handler);
  return {
    async waitHeld() { await expect.poll(() => held !== undefined, { timeout: 20_000 }).toBe(true); },
    async release() {
      const current = held!;
      await current.route.fulfill({ status: current.status, headers: current.headers, body: current.body });
      await page.unroute("**/*", handler);
    },
  };
}

async function cookieToken(context: BrowserContext): Promise<string | null> {
  return (await activeCredentialCookie(context))?.value.split(".").at(-1) ?? null;
}

async function signOutTab(tab: Page, url: string) {
  await tab.goto(url);
  await tab.getByRole("button", { name: "账户菜单", exact: true }).click();
  await tab.getByRole("menuitem", { name: "退出登录", exact: true }).click();
}

test("a delayed final cookie deletion for account A (logout or login-page cleanup) leaves account B signed in", async ({ browser }) => {
  test.setTimeout(240_000);
  const stack = await accountStack("session-cleanup");
  const { base, webUrl, admin, networkId } = stack;
  const context = await browser.newContext();
  try {
    const a = { email: "owner-a@example.test", password: testPassword("a") };
    await sdk.claimFirstOwner(base, admin, networkId, { kind: "new_account", ...a });
    const aSession = await sdk.loginAccount(base, a);
    const b = await invitedAccount(base, aSession.session_token, networkId, "owner-b@example.test", "owner");
    await sdk.logoutAccount(base, aSession.session_token);
    const tabA = await context.newPage();
    await tabA.addInitScript(() => { Reflect.deleteProperty(window, "BroadcastChannel"); });
    const tabB = await context.newPage();
    const machines = `${webUrl}/${networkId}/machines`;

    await test.step("A's logout: the response deleting A's cookie arrives after B signed in", async () => {
      await tabA.goto(`${webUrl}/login`);
      await signIn(tabA, a.email, a.password);
      const held = await holdFinalDeletion(tabA);
      await signOutTab(tabA, machines);
      await held.waitHeld();
      // A is revoked on the controller but the browser still holds A's cookie; B signs in for real.
      await tabB.goto(machines);
      await expect(tabB).toHaveURL(/\/login/);
      await signIn(tabB, b.email, b.password);
      const bToken = await cookieToken(context);
      expect(bToken?.startsWith("mws_"), "B has a user session").toBe(true);
      await held.release();
      expect.soft(await cookieToken(context) === bToken, "B keeps its cookie").toBe(true);
      await sdk.getAccountSession(base, bToken!);
      await tabB.goto(machines);
      await expect.soft(tabB).toHaveURL(machines);
    });

    await test.step("login-page cleanup of A's rejected cookie, delivered after B signed in", async () => {
      // Independent of the previous step's outcome: start from a signed-out browser.
      await context.clearCookies();
      await tabB.goto(`${webUrl}/login`);
      await signIn(tabB, a.email, a.password);
      const aToken = await cookieToken(context);
      // A's session ends on the controller (e.g. revoked elsewhere); A's tab hits the rejection.
      await sdk.logoutAccount(base, aToken!);
      const held = await holdFinalDeletion(tabA);
      await tabA.goto(machines);
      await expect(tabA).toHaveURL(/\/login\?reason=expired/);
      await held.waitHeld();
      await tabB.goto(`${webUrl}/login`);
      await expect(tabB.getByRole("button", { name: "使用邮箱登录", exact: true })).toBeVisible();
      await signIn(tabB, b.email, b.password);
      const bToken = await cookieToken(context);
      await held.release();
      expect.soft(await cookieToken(context) === bToken, "B keeps its cookie").toBe(true);
      await sdk.getAccountSession(base, bToken!);
    });

    await test.step("normal logouts remove the credential itself, for a user and for the administrator token", async () => {
      await context.clearCookies();
      await tabB.goto(`${webUrl}/login`);
      await signIn(tabB, b.email, b.password);
      await signOutTab(tabB, machines);
      await expect(tabB).toHaveURL(/\/login/);
      expect.soft(await credentialCookies(context), "no user credential left").toHaveLength(0);
      await tabB.getByLabel("管理员令牌", { exact: true }).fill(admin);
      await tabB.getByRole("button", { name: "登录", exact: true }).click();
      await expect(tabB).not.toHaveURL(/\/login/);
      await signOutTab(tabB, machines);
      await expect(tabB).toHaveURL(/\/login/);
      const left = await context.cookies();
      expect.soft(left.some(cookie => cookie.value.includes(admin)), "no administrator token left in any cookie").toBe(false);
      await tabB.goto(machines);
      await expect(tabB).toHaveURL(/\/login/);
    });
  } finally {
    await context.close(); await stack.stop();
  }
});
