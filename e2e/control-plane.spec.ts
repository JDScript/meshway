import { expect, test } from "@playwright/test";
import { execFile, spawn } from "node:child_process";
import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { promisify } from "node:util";
import { listNodes } from "../packages/control-client/src/index.ts";

const root = resolve(import.meta.dirname, "..");
const run = promisify(execFile);
const executable = (name: string) =>
  join(root, "target", "debug", `${name}${process.platform === "win32" ? ".exe" : ""}`);

async function unusedPort(): Promise<number> {
  const server = createServer();
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (address === null || typeof address === "string") {
    server.close();
    throw new Error("Expected a TCP address");
  }
  await new Promise<void>((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
  return address.port;
}

function service(command: string, args: string[], environment: NodeJS.ProcessEnv = process.env) {
  const child = spawn(command, args, { cwd: root, env: environment, stdio: ["ignore", "pipe", "pipe"] });
  let output = "";
  for (const stream of [child.stdout, child.stderr]) {
    stream.on("data", (chunk: Buffer) => { output = `${output}${chunk.toString()}`.slice(-32_768); });
  }
  let launchError: Error | undefined;
  const exited = new Promise<void>((resolve) => {
    child.once("exit", () => resolve());
    child.once("error", (error) => { launchError = error; resolve(); });
  });
  return {
    async ready(url: string, status: number) {
      await Promise.race([
        expect.poll(async () => {
          try {
            const response = await fetch(url, { redirect: "manual", signal: AbortSignal.timeout(1000) });
            await response.body?.cancel();
            return response.status;
          } catch { return 0; }
        }, { timeout: 25_000, message: `Service readiness: ${url}` }).toBe(status),
        exited.then(() => { throw new Error(`Service stopped during startup: ${launchError?.message ?? output}`); }),
      ]);
    },
    async stop() {
      if (child.exitCode !== null || child.signalCode !== null || launchError) return;
      child.kill("SIGINT");
      const deadline = new AbortController();
      try {
        await Promise.race([
          exited,
          delay(5000, undefined, { signal: deadline.signal }).then(async () => {
            child.kill("SIGKILL");
            await exited;
          }),
        ]);
      } finally { deadline.abort(); }
    },
  };
}

test("administrator session, enrollment, durable identity and revocation cross the real stack", async ({ page, browser }) => {
  const browserErrors: string[] = [];
  page.on("pageerror", (error) => browserErrors.push(error.message));
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-e2e-"));
  const dataDirectory = join(temporary, "controller");
  const agentState = join(temporary, "agent.json");
  const nodeName = "-edge's nyc 🇨🇳";
  const controllerPort = await unusedPort();
  const webPort = await unusedPort();
  const controllerUrl = `http://127.0.0.1:${controllerPort}`;
  const webUrl = `http://127.0.0.1:${webPort}`;
  const startController = () => service(executable("meshway-controller"), [
    "serve", "--data-dir", dataDirectory, "--bind", `127.0.0.1:${controllerPort}`,
  ]);
  let controller = startController();
  let web: ReturnType<typeof service> | undefined;
  const anonymous = await browser.newContext();
  try {
    await controller.ready(`${controllerUrl}/api/v1/overview`, 401);
    const adminToken = (await readFile(join(dataDirectory, "admin.token"), "utf8")).trim();
    expect(adminToken.length).toBeGreaterThan(30);
    web = service(process.execPath, [
      join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"),
      "--hostname", "127.0.0.1", "--port", String(webPort),
    ], {
      ...process.env,
      MESHWAY_CONTROLLER_URL: controllerUrl,
      MESHWAY_WEB_COOKIE_SECURE: "auto",
      NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);

    await test.step("anonymous reads and incorrect credentials cannot establish an admin session", async () => {
      await page.goto(`${webUrl}/nodes`);
      await expect(page).toHaveURL(/\/login(?:\?|$)/);
      await page.getByLabel("管理员令牌", { exact: true }).fill("not-an-administrator-token");
      await Promise.all([
        page.waitForResponse((response) => response.request().method() === "POST" && new URL(response.url()).pathname === "/login"),
        page.getByRole("button", { name: "登录", exact: true }).click(),
      ]);
      await expect(page.getByRole("button", { name: "登录", exact: true })).toBeEnabled();
      await expect(page).toHaveURL(/\/login(?:\?|$)/);
      await expect(page.getByLabel("管理员令牌", { exact: true })).toBeVisible();
      expect((await page.context().cookies()).filter((cookie) => cookie.httpOnly)).toHaveLength(0);
    });

    await test.step("login keeps the administrative credential in an HttpOnly session", async () => {
      await page.getByLabel("管理员令牌", { exact: true }).fill(adminToken);
      await page.getByRole("button", { name: "登录", exact: true }).click();
      await expect(page).not.toHaveURL(/\/login(?:\?|$)/);
      expect(await page.content()).not.toContain(adminToken);
      expect(await page.evaluate(() => document.cookie)).not.toContain(adminToken);
      const sessionCookies = (await page.context().cookies()).filter((cookie) => cookie.httpOnly);
      expect(sessionCookies.length).toBeGreaterThan(0);
      for (const cookie of sessionCookies) {
        expect(cookie.sameSite).toBe("Strict");
        expect(cookie.secure).toBe(false);
        expect(cookie.expires).toBeGreaterThan(Date.now() / 1000);
        expect(cookie.expires).toBeLessThan(Date.now() / 1000 + 9 * 3600);
      }
    });

    let enrollmentToken = "";
    let enrollmentArguments: string[] = [];
    await test.step("mint a token and reject an anonymous replay of the same server action", async () => {
      await page.goto(`${webUrl}/enrollment`);
      const mutation = page.waitForRequest((request) =>
        request.method() === "POST" && new URL(request.url()).pathname === "/enrollment",
      );
      await page.getByRole("button", { name: "创建注册令牌", exact: true }).click();
      const request = await mutation;
      const token = page.getByTestId("enrollment-token");
      await expect(token).toBeVisible();
      enrollmentToken = (await token.innerText()).trim();
      expect(enrollmentToken.length).toBeGreaterThan(30);
      expect(await page.content()).not.toContain(adminToken);

      const originalHeaders = await request.allHeaders();
      const headers = Object.fromEntries(Object.entries(originalHeaders).filter(([name]) =>
        ["content-type", "next-action", "next-router-state-tree"].includes(name.toLowerCase()),
      ));
      const body = request.postData();
      if (!body) throw new Error("Expected server action body for the authenticated mutation");
      const response = await anonymous.request.post(request.url(), {
        headers: { ...headers, origin: webUrl }, data: body, maxRedirects: 0,
      });
      const redirect = response.headers()["x-action-redirect"] ?? response.headers().location ?? "";
      expect(redirect).toContain("/login");

      const nameInput = page.getByLabel("节点名称", { exact: true });
      const astralName = "😀".repeat(128);
      await nameInput.fill(astralName);
      await expect(nameInput).toHaveValue(astralName);
      await nameInput.fill("😀".repeat(129));
      await expect(nameInput).toHaveAttribute("aria-invalid", "true");
      await nameInput.fill(nodeName);
      const displayedCommand = await page.locator("pre").filter({ hasText: "meshway-agent" }).first().innerText();
      // Parse the actual copied command with the shell, without executing cargo.
      // Only the state destination changes so the real CLI cannot touch user state.
      const parsed = await run("/bin/sh", ["-c", `set -- ${displayedCommand}; printf '%s\\0' "$@"`], { cwd: temporary });
      const argumentsWithCargo = parsed.stdout.split("\0").slice(0, -1);
      expect(argumentsWithCargo[0]).toBe("cargo");
      const separator = argumentsWithCargo.indexOf("--");
      expect(separator).toBeGreaterThan(0);
      enrollmentArguments = argumentsWithCargo.slice(separator + 1);
      const stateFlag = enrollmentArguments.findIndex((argument) => argument === "--state" || argument.startsWith("--state="));
      expect(stateFlag).toBeGreaterThan(0);
      if (enrollmentArguments[stateFlag] === "--state") enrollmentArguments[stateFlag + 1] = agentState;
      else enrollmentArguments[stateFlag] = `--state=${agentState}`;
    });

    let nodeId = "";
    await test.step("a real generated Rust client enrolls and the Web renders its identity", async () => {
      await run(executable("meshway-agent"), enrollmentArguments, { cwd: root, timeout: 15_000 });
      await run(executable("meshway-agent"), ["heartbeat", "--state", agentState], { timeout: 15_000 });
      const nodes = await listNodes(controllerUrl, adminToken);
      expect(nodes.nodes).toHaveLength(1);
      const node = nodes.nodes[0];
      if (!node) throw new Error("Expected one enrolled node");
      nodeId = node.id;
      expect(node.name).toBe(nodeName);
      expect(node.revoked_at).toBeNull();
      if (process.platform !== "win32") expect((await stat(agentState)).mode & 0o777).toBe(0o600);
      await page.goto(`${webUrl}/nodes`);
      await expect(page.getByRole("row").filter({ hasText: nodeName })).toBeVisible();
      expect(await page.content()).not.toContain(adminToken);
    });

    await test.step("controller outage is visible and restart preserves both admin and node identity", async () => {
      await controller.stop();
      await page.getByRole("button", { name: "重新读取", exact: true }).click();
      await expect(page.getByText("无法读取 controller", { exact: true })).toBeVisible();
      controller = startController();
      await controller.ready(`${controllerUrl}/api/v1/overview`, 401);
      await run(executable("meshway-agent"), ["heartbeat", "--state", agentState], { timeout: 15_000 });
      expect((await listNodes(controllerUrl, adminToken)).nodes.map((node) => node.id)).toEqual([nodeId]);
      await page.goto(`${webUrl}/nodes`);
      await expect(page.getByRole("row").filter({ hasText: nodeName })).toBeVisible();
    });

    await test.step("Web revocation rejects the agent credential and logout closes the browser session", async () => {
      await page.getByRole("row").filter({ hasText: nodeName }).getByRole("button", { name: /撤销/ }).click();
      const confirmation = page.getByRole("alertdialog");
      if (await confirmation.isVisible()) await confirmation.getByRole("button", { name: /撤销/ }).click();
      await expect(page.getByRole("row").filter({ hasText: nodeName })).toContainText("已撤销");
      const node = (await listNodes(controllerUrl, adminToken)).nodes.find((node) => node.id === nodeId);
      expect(node?.revoked_at).toBeTruthy();
      await expect(run(executable("meshway-agent"), ["heartbeat", "--state", agentState], { timeout: 15_000 }))
        .rejects.toThrow(/401|403|revoked|forbidden|unauthorized/i);
      await page.setViewportSize({ width: 390, height: 844 });
      expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
      await page.getByRole("button", { name: "退出登录", exact: true }).click();
      await expect(page).toHaveURL(/\/login(?:\?|$)/);
      await page.goto(`${webUrl}/nodes`);
      await expect(page).toHaveURL(/\/login(?:\?|$)/);
      expect(browserErrors).toEqual([]);
    });
  } finally {
    await anonymous.close();
    await web?.stop();
    await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
