import { expect, test, type BrowserContext, type Page, type Request, type Route } from "@playwright/test";
import * as sdk from "./network-client.ts";
import { accountStack, approve, deviceRequest, invitedAccount, replayHeaders, signIn, testPassword } from "./accounts.ts";

// Two tabs of one browser (one cookie jar). Tab A renders forms and drafts as
// account A; tab B then really signs in as account B. Anything tab A rendered
// must not be carried out with B's cookie, and nothing late from A may sign B out.

const consoleCookie = async (context: BrowserContext) =>
  (await context.cookies()).find(cookie => cookie.name.startsWith("meshway_console_session") && cookie.value !== "");

/** The bearer inside the browser's HttpOnly cookie; fixture memory only, never printed. */
async function cookieToken(context: BrowserContext): Promise<string | null> {
  return (await consoleCookie(context))?.value.split(".").at(-1) ?? null;
}

/** Captures one server-action POST to `path` from `page` without letting it reach the server. */
async function capture(page: Page, path: string, trigger: () => Promise<void>) {
  let captured: Request | undefined;
  const handler = async (route: Route) => {
    if (route.request().method() !== "POST") return route.continue();
    captured = route.request();
    await route.abort();
  };
  await page.route(`**${path}*`, handler);
  await trigger();
  await expect.poll(() => captured !== undefined).toBe(true);
  await page.unroute(`**${path}*`, handler);
  const request = captured!;
  return { url: request.url(), data: request.postData() ?? "", headers: await replayHeaders(request) };
}

/** Holds one server-action POST to `path` until `release()` sends it unchanged (with the cookies it was issued with). */
async function hold(page: Page, path: string) {
  let held: Route | undefined;
  const handler = async (route: Route) => {
    if (route.request().method() !== "POST" || held) return route.continue();
    held = route;
  };
  await page.route(`**${path}*`, handler);
  return {
    async waitHeld() { await expect.poll(() => held !== undefined).toBe(true); },
    async release() {
      const response = page.waitForResponse(r => r.request().method() === "POST" && new URL(r.url()).pathname === path);
      // Sent exactly as issued, with the cookie A had at the time.
      await held!.continue({ headers: await held!.request().allHeaders() });
      const result = await response;
      await page.unroute(`**${path}*`, handler);
      return result;
    },
  };
}

async function switchTo(tab: Page, webUrl: string, networkId: string, email: string, password: string) {
  await tab.goto(`${webUrl}/${networkId}/machines`);
  await tab.getByRole("button", { name: "账户菜单", exact: true }).click();
  await tab.getByRole("menuitem", { name: "退出登录", exact: true }).click();
  await expect(tab).toHaveURL(/\/login(?:\?|$)/);
  await signIn(tab, email, password);
}

async function twoOwners(browser: import("@playwright/test").Browser, prefix: string) {
  const stack = await accountStack(prefix);
  const { base, admin, networkId } = stack;
  const a = { email: "owner-a@example.test", password: testPassword("a") };
  await sdk.claimFirstOwner(base, admin, networkId, { kind: "new_account", ...a });
  const aSession = await sdk.loginAccount(base, a);
  const b = await invitedAccount(base, aSession.session_token, networkId, "owner-b@example.test", "owner");
  await sdk.logoutAccount(base, aSession.session_token);
  const context = await browser.newContext();
  // Tab A never hears tab B's broadcast, so only the server can tell.
  const tabA = await context.newPage();
  await tabA.addInitScript(() => { Reflect.deleteProperty(window, "BroadcastChannel"); });
  await tabA.goto(`${stack.webUrl}/login`);
  await signIn(tabA, a.email, a.password);
  const tabB = await context.newPage();
  return { stack, a, b, context, tabA, tabB };
}

test("a page rendered for account A cannot approve or save with account B's cookie; B's own render succeeds", async ({ browser }) => {
  test.setTimeout(240_000);
  const { stack, a, b, context, tabA, tabB } = await twoOwners(browser, "session-fence");
  const { base, webUrl, admin, networkId } = stack;
  try {

    const device = deviceRequest("Stale approval");
    await tabA.goto(device.url(webUrl));
    await expect(tabA.getByText(a.email, { exact: true })).toBeVisible();
    const oldApproval = await capture(tabA, "/authorize-device", () => tabA.getByRole("button", { name: "批准接入", exact: true }).click());

    await tabA.goto(`${webUrl}/${networkId}/access`);
    await tabA.getByRole("button", { name: "JSONC", exact: true }).click();
    const before = await sdk.getMeshAccess(base, admin);
    const staleSource = `// account A draft\n${JSON.stringify(before.policy)}`;
    await tabA.getByRole("textbox", { name: "访问授权 JSONC", exact: true }).fill(staleSource);
    const oldSave = await capture(tabA, `/${networkId}/access`, () => tabA.getByTestId("access-save").click());

    // A second tab of A keeps an unsent draft (and cannot hear broadcasts either).
    const staleEditor = await context.newPage();
    await staleEditor.addInitScript(() => { Reflect.deleteProperty(window, "BroadcastChannel"); });
    await staleEditor.goto(`${webUrl}/${networkId}/access`);
    await staleEditor.getByRole("button", { name: "JSONC", exact: true }).click();
    await staleEditor.getByRole("textbox", { name: "访问授权 JSONC", exact: true }).fill(`// unsent A draft\n${JSON.stringify(before.policy)}`);

    // Tab B really signs in as B in the same browser.
    await switchTo(tabB, webUrl, networkId, b.email, b.password);
    const bToken = await cookieToken(context);
    expect(bToken?.startsWith("mws_"), "B has a user session").toBe(true);

    await test.step("A's old approval and draft save, replayed with B's cookie, do nothing", async () => {
      const nodes = (await sdk.listNodes(base, admin)).nodes.length;
      const approval = await context.request.post(oldApproval.url, { headers: { ...oldApproval.headers, origin: webUrl }, data: oldApproval.data, maxRedirects: 0 });
      expect.soft(approval.headers()["x-action-redirect"] ?? approval.headers().location ?? "").not.toContain("meshway://");
      expect.soft(await approval.text()).not.toMatch(/meshway:\/\/enroll\/callback\?code=/);
      const save = await context.request.post(oldSave.url, { headers: { ...oldSave.headers, origin: webUrl }, data: oldSave.data, maxRedirects: 0 });
      expect(save.status()).toBeLessThan(500);
      const after = await sdk.getMeshAccess(base, admin);
      expect.soft(after.revision).toBe(before.revision);
      expect.soft(after.source_jsonc).not.toBe(staleSource);
      expect.soft((await sdk.listNodes(base, admin)).nodes.length).toBe(nodes);
      // B stays signed in.
      expect.soft(await cookieToken(context) === bToken, "B keeps its cookie").toBe(true);
      await sdk.getAccountSession(base, bToken!);
    });

    await test.step("tab A's own stale editor, saved for real in the browser, is refused and points at the changed sign-in", async () => {
      await staleEditor.bringToFront();
      await staleEditor.getByTestId("access-save").click();
      await expect.soft(staleEditor).toHaveURL(/\/session-changed/);
      await expect.soft(staleEditor.getByRole("heading", { name: "登录已变化" })).toBeVisible();
      expect.soft((await sdk.getMeshAccess(base, admin)).source_jsonc ?? "").not.toContain("unsent A draft");
      expect.soft(await cookieToken(context) === bToken, "B keeps its cookie").toBe(true);
    });

    await test.step("the same operations rendered for B succeed as B", async () => {
      await tabB.goto(device.url(webUrl));
      await expect(tabB.getByText(b.email, { exact: true })).toBeVisible();
      const { callback } = await approve(tabB);
      const code = callback?.searchParams.get("code"); if (!code) throw new Error("Missing code for B");
      const node = await sdk.exchangeDevice(webUrl, code, device.binding);
      const membership = (await sdk.getAccountSession(base, bToken!)).memberships[0];
      const owned = (await sdk.listNodeOwnership(base, bToken!, networkId)).nodes.find(item => item.node_id === node.node.id);
      expect(owned?.membership_id).toBe(membership?.membership_id);
      await tabB.goto(`${webUrl}/${networkId}/access`);
      await tabB.getByRole("button", { name: "JSONC", exact: true }).click();
      const source = `// account B save\n${JSON.stringify(before.policy)}`;
      await tabB.getByRole("textbox", { name: "访问授权 JSONC", exact: true }).fill(source);
      await tabB.getByTestId("access-save").click();
      await expect(tabB.getByTestId("access-save-status")).toContainText("已保存");
      expect((await sdk.getMeshAccess(base, admin)).source_jsonc).toBe(source);
    });

    await test.step("editing the unsigned cookie's session ID cannot make A's credential pass as B's rendered page", async () => {
      await tabB.goto(`${webUrl}/${networkId}/access`);
      await tabB.getByRole("button", { name: "JSONC", exact: true }).click();
      const base0 = await sdk.getMeshAccess(base, admin);
      await tabB.getByRole("textbox", { name: "访问授权 JSONC", exact: true }).fill(`// B-rendered\n${JSON.stringify(base0.policy)}`);
      const bRendered = await capture(tabB, `/${networkId}/access`, () => tabB.getByTestId("access-save").click());
      const bCookie = await consoleCookie(context);
      // A signs in again for real (B's tab signs out), then A's cookie is given B's session ID.
      await switchTo(tabA, webUrl, networkId, a.email, a.password);
      const aCookie = await consoleCookie(context);
      if (!bCookie || !aCookie) throw new Error("Missing console cookie");
      const forged = aCookie.value.split(".");
      forged[3] = bCookie.value.split(".")[3] ?? "";
      await context.addCookies([{ ...aCookie, value: forged.join(".") }]);
      const replay = await context.request.post(bRendered.url, { headers: { ...bRendered.headers, origin: webUrl }, data: bRendered.data, maxRedirects: 0 });
      expect(replay.headers()["x-action-redirect"] ?? "").toContain("/session-changed");
      expect((await sdk.getMeshAccess(base, admin)).revision).toBe(base0.revision);
      await context.addCookies([aCookie]);
    });

  } finally {
    await context.close(); await stack.stop();
  }
});

test("late 401 or logout responses for account A leave account B signed in", async ({ browser }) => {
  test.setTimeout(240_000);
  const { stack, a, b, context, tabA, tabB } = await twoOwners(browser, "session-late");
  const { base, webUrl, admin, networkId } = stack;
  try {
    const before = await sdk.getMeshAccess(base, admin);
    await test.step("a late 401 for A's held request does not clear B's cookie", async () => {
      await switchTo(tabB, webUrl, networkId, a.email, a.password);
      const aToken = await cookieToken(context);
      await tabA.goto(`${webUrl}/${networkId}/access`);
      await tabA.getByRole("button", { name: "JSONC", exact: true }).click();
      await tabA.getByRole("textbox", { name: "访问授权 JSONC", exact: true }).fill(`// held A\n${JSON.stringify(before.policy)}`);
      const held = await hold(tabA, `/${networkId}/access`);
      await tabA.getByTestId("access-save").click();
      await held.waitHeld();
      // A is signed out (revoked on the controller), then B signs in.
      await switchTo(tabB, webUrl, networkId, b.email, b.password);
      const bAgain = await cookieToken(context);
      expect(bAgain !== aToken, "B replaced A").toBe(true);
      await held.release();
      expect.soft(await cookieToken(context) === bAgain, "B keeps its cookie").toBe(true);
      await sdk.getAccountSession(base, bAgain!);
    });

    await test.step("a late logout response for A does not clear B's cookie", async () => {
      await switchTo(tabB, webUrl, networkId, a.email, a.password);
      await tabA.goto(`${webUrl}/${networkId}/machines`);
      await tabA.getByRole("button", { name: "账户菜单", exact: true }).click();
      const held = await hold(tabA, `/${networkId}/machines`);
      await tabA.getByRole("menuitem", { name: "退出登录", exact: true }).click();
      await held.waitHeld();
      await switchTo(tabB, webUrl, networkId, b.email, b.password);
      const bAgain = await cookieToken(context);
      await held.release();
      expect.soft(await cookieToken(context) === bAgain, "B keeps its cookie").toBe(true);
      await sdk.getAccountSession(base, bAgain!);
      await tabB.goto(`${webUrl}/${networkId}/machines`);
      await expect(tabB).toHaveURL(`${webUrl}/${networkId}/machines`);
    });
  } finally {
    await context.close(); await stack.stop();
  }
});
