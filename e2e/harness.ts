import { expect, type Locator, type Page } from "@playwright/test";
import { execFile, spawn } from "node:child_process";
import { createServer } from "node:net";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { promisify } from "node:util";

export const root = resolve(import.meta.dirname, "..");
export const run = promisify(execFile);
export const executable = (name: string) =>
  join(root, "target", "debug", `${name}${process.platform === "win32" ? ".exe" : ""}`);

export async function expandDetails(scope: Page | Locator, title: string) {
  const summary = scope.locator("summary").filter({ hasText: title }).first();
  await expect(summary).toBeVisible();
  const details = summary.locator("..");
  if (await details.getAttribute("open") === null) await summary.click();
  await expect(details).toHaveAttribute("open", "");
}

export async function unusedPort(): Promise<number> {
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

export function service(command: string, args: string[], environment: NodeJS.ProcessEnv = process.env) {
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
      url = url.replace(/\/api\/v1\/overview$/, "/api/v1/networks");
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

/** Console pages live under `/<network_id>`; fixtures use the controller's first network. */
export async function consoleAt(webUrl: string, base: string, admin: string) {
  const { listNetworks } = await import("../packages/control-client/src/index.ts");
  const network = (await listNetworks(base, admin)).networks[0];
  if (!network) throw new Error("Fixture controller has no network");
  const prefix = `/${network.network_id}`;
  return {
    networkId: network.network_id,
    /** Absolute URL of a console page, e.g. at("/machines"). */
    at: (path = "") => `${webUrl}${prefix}${path}`,
    /** Path name as seen by request matchers, e.g. path("/access"). */
    path: (path = "") => `${prefix}${path}`,
  };
}

/** Sidebar width on desktop (≥ 768px); on mobile the sidebar is a drawer. */
export const SIDEBAR_WIDTH = 240;

/** Expected page gutter inside a main column of `inset` px at a viewport `width` px wide. */
export function consoleGutter(width: number, inset: number): number {
  return width < 768 ? 16 : Math.max(32, (inset - 1216) / 2 + 32);
}

/**
 * The page body's content column lines up with the top bar at every width:
 * the sidebar toggle on the left, the global actions on the right.
 */
export async function contentAlignment(page: Page) {
  return page.evaluate(() => {
    const header = document.querySelector('[data-slot="page-header"]');
    const inset = document.querySelector('[data-slot="sidebar-inset"]');
    const toggle = document.querySelector('[data-slot="console-topbar"] [data-sidebar="trigger"]');
    const actions = document.querySelector('[data-slot="console-topbar-actions"]');
    if (!header || !inset || !toggle || !actions) throw new Error("Missing console shell");
    const h = header.getBoundingClientRect();
    const i = inset.getBoundingClientRect();
    // The ghost toggle sits 6px outside the column so its icon lines up with the text.
    return {
      left: Math.round(h.left - toggle.getBoundingClientRect().left - 6),
      right: Math.round(actions.getBoundingClientRect().right - h.right),
      gutter: Math.round(h.left - i.left),
      inset: Math.round(i.width),
      overflow: document.documentElement.scrollWidth > innerWidth,
    };
  });
}

/** No horizontal page scroll; on failure names the elements that stick out. */
export async function expectNoOverflow(page: Page) {
  const offenders = await page.evaluate(() => {
    if (document.documentElement.scrollWidth <= innerWidth) return [];
    const scrollers = (element: Element | null): boolean => {
      for (let current = element?.parentElement; current; current = current.parentElement) {
        const style = getComputedStyle(current);
        if (["auto", "scroll", "hidden", "clip"].includes(style.overflowX)) return true;
      }
      return false;
    };
    return [...document.querySelectorAll("body *")]
      .filter(element => element.getBoundingClientRect().right > innerWidth + 1 && !scrollers(element))
      .slice(0, 8)
      .map(element => `${element.tagName.toLowerCase()}[${element.getAttribute("data-slot") ?? element.getAttribute("data-testid") ?? ""}] ${(element.textContent ?? "").slice(0, 40)}`);
  });
  expect(offenders).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
}
