import { expect, test } from "@playwright/test";
import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  createEnrollmentToken,
  getPolicy,
  listNodes,
  previewPolicy,
  revokeNode,
  savePolicy,
  setNodeCountry,
  type FlowQuery,
  type NodeSummary,
  type RoutingPolicy,
} from "./network-client.ts";
import { executable, root, run, service, unusedPort } from "./harness.ts";
import { discardPolicyDraft, editRuleId, selectNode } from "./web-actions.ts";

function named(nodes: NodeSummary[], name: string): NodeSummary {
  const node = nodes.find((node) => node.name === name);
  if (!node) throw new Error(`Missing enrolled node ${name}`);
  return node;
}

test("routing intent, conditional editing and persisted configuration receipts cross the real stack", async ({ page, browser }) => {
  const temporary = await mkdtemp(join(process.env.MESHWAY_E2E_TMPDIR ?? tmpdir(), "meshway-routing-e2e-"));
  const dataDirectory = join(temporary, "controller");
  const controllerPort = await unusedPort();
  const webPort = await unusedPort();
  const controllerUrl = `http://127.0.0.1:${controllerPort}`;
  const webUrl = `http://127.0.0.1:${webPort}`;
  const startController = () => service(executable("meshway-controller"), [
    "serve", "--data-dir", dataDirectory, "--bind", `127.0.0.1:${controllerPort}`,
    "--ipv4-prefix", "100.96.0.0/29", "--ipv6-prefix", "fd42:6d65:7368::/64",
  ]);
  let controller = startController();
  let web: ReturnType<typeof service> | undefined;
  const anonymous = await browser.newContext();
  const browserErrors: string[] = [];
  page.on("pageerror", (error) => browserErrors.push(error.message));
  try {
    await controller.ready(`${controllerUrl}/api/v1/overview`, 401);
    const admin = (await readFile(join(dataDirectory, "admin.token"), "utf8")).trim();
    for (const name of ["nyc", "lax", "sha"]) {
      const grant = await createEnrollmentToken(controllerUrl, admin, { expires_in_seconds: 600 });
      await run(executable("meshway-agent"), [
        "enroll", "--controller", controllerUrl, "--token", grant.token,
        "--name", name, "--state", join(temporary, `${name}.json`),
      ], { timeout: 15_000 });
    }
    const inventory = (await listNodes(controllerUrl, admin)).nodes;
    const nyc = named(inventory, "nyc");
    const lax = named(inventory, "lax");
    const sha = named(inventory, "sha");
    expect(new Set(inventory.map((node) => node.mesh_ipv4)).size).toBe(3);
    expect(new Set(inventory.map((node) => node.mesh_ipv6)).size).toBe(3);
    for (const entry of [{ node: nyc, country: "US" }, { node: lax, country: "US" }, { node: sha, country: "CN" }]) {
      const snapshot = await getPolicy(controllerUrl, admin);
      await setNodeCountry(controllerUrl, admin, {
        node_id: entry.node.id, country: entry.country, expected_revision: snapshot.revision,
      });
    }
    const policy: RoutingPolicy = {
      default_action: "reject",
      rules: [
        {
          id: "nyc-to-sha",
          source: { node_ids: [nyc.id], countries: [] },
          destination_scope: "mesh",
          destination_any: [{ kind: "node", value: sha.id }],
          action: { kind: "route", via: [lax.id], exit_node_id: null },
        },
        {
          id: "us-to-cn",
          source: { node_ids: [], countries: ["US"] },
          destination_scope: "internet",
          destination_any: [
            { kind: "domain_suffix", value: "example.cn" },
            { kind: "cidr", value: "203.0.113.0/24" },
            { kind: "country", value: "CN" },
          ],
          action: { kind: "route", via: [], exit_node_id: lax.id },
        },
      ],
    };
    const meshFlow: FlowQuery = {
      source_node_id: nyc.id, destination_node_id: sha.id,
      destination_ip: null, domain: null, destination_country: null,
    };

    await test.step("preview evaluates logical hops and suffix boundaries without saving", async () => {
      const before = await getPolicy(controllerUrl, admin);
      const mesh = await previewPolicy(controllerUrl, admin, { policy, flow: meshFlow });
      expect(mesh.valid).toBe(true);
      expect(mesh.decision.path).toEqual([nyc.id, lax.id, sha.id]);
      const literal = await previewPolicy(controllerUrl, admin, {
        policy, flow: { ...meshFlow, destination_node_id: null, destination_ip: sha.mesh_ipv6 },
      });
      expect(literal.decision.path).toEqual([nyc.id, lax.id, sha.id]);
      const internetFlow: FlowQuery = { ...meshFlow, destination_node_id: null, domain: "API.Example.CN." };
      const internet = await previewPolicy(controllerUrl, admin, { policy, flow: internetFlow });
      expect(internet.decision.path).toEqual([nyc.id, lax.id]);
      expect(internet.decision.exit_node_id).toBe(lax.id);
      const boundary = await previewPolicy(controllerUrl, admin, {
        policy, flow: { ...internetFlow, domain: "badexample.cn" },
      });
      expect(boundary.decision.action).toBe("reject");
      for (const address of [sha.mesh_ipv4, "100.96.0.6", "203.0.113.10"]) {
        const mapped = await previewPolicy(controllerUrl, admin, {
          policy: { ...policy, default_action: "direct" },
          flow: { ...meshFlow, destination_node_id: null, destination_ip: `::ffff:${address}` },
        });
        expect(mapped.valid).toBe(false);
        expect(mapped.decision.action).toBe("invalid");
        expect(mapped.decision.path).toEqual([]);
      }
      expect(await getPolicy(controllerUrl, admin)).toEqual(before);
      await savePolicy(controllerUrl, admin, { expected_revision: before.revision, policy });
    });

    web = service(process.execPath, [
      join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"),
      "--hostname", "127.0.0.1", "--port", String(webPort),
    ], {
      ...process.env, MESHWAY_CONTROLLER_URL: controllerUrl,
      MESHWAY_WEB_COOKIE_SECURE: "auto", NEXT_TELEMETRY_DISABLED: "1",
    });
    await web.ready(`${webUrl}/login`, 200);
    await page.goto(`${webUrl}/login`);
    await page.getByLabel("管理员令牌", { exact: true }).fill(admin);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).not.toHaveURL(/\/login(?:\?|$)/);

    await test.step("new topology and monitoring views are protected and retain truthful path snapshots", async () => {
      const stranger = await anonymous.newPage();
      try {
        for (const path of ["/topology", "/monitoring"]) {
          await stranger.goto(`${webUrl}${path}`);
          await expect(stranger).toHaveURL(/\/login(?:\?|$)/);
          expect(await stranger.content()).not.toContain(nyc.id);
        }
      } finally { await stranger.close(); }

      await page.goto(`${webUrl}/topology`);
      await selectNode(page, "preview-source", /nyc/);
      await selectNode(page, "preview-target", /sha/);
      await page.getByRole("button", { name: "规划路径", exact: true }).click();
      const path = page.getByRole("list", { name: "规划路径", exact: true });
      await expect(path).toContainText("lax");
      const steps = await path.getByRole("listitem").allTextContents();
      expect(steps).toHaveLength(3);
      expect(steps[0]).toContain("nyc");
      expect(steps[1]).toContain("lax");
      expect(steps[2]).toContain("sha");
      await selectNode(page, "preview-target", /^外部目标（域名 \/ IP）$/);
      await page.getByLabel("域名", { exact: true }).fill("allowed.example.cn");
      await page.getByRole("button", { name: "规划路径", exact: true }).click();
      await expect(page.getByTestId("route-preview")).toContainText("allowed.example.cn");
      await page.getByLabel("域名", { exact: true }).fill("denied.invalid");
      await expect(page.getByTestId("route-preview")).toContainText("流量已修改");
      await expect(page.getByTestId("route-preview")).toContainText("allowed.example.cn");
      await expect(page.getByTestId("route-preview")).not.toContainText("denied.invalid");
      expect(await page.content()).not.toContain(admin);
    });

    await test.step("the typed editor previews a draft, then saves it under its original revision", async () => {
      await page.goto(`${webUrl}/routing`);
      await expect(page.getByTestId("policy-editor")).toBeVisible();
      await expect(page.locator('[data-slot="draft-action-bar"]')).toBeHidden();
      await expect(page.getByTestId("policy-rule")).toHaveCount(2);
      await expect(page.getByTestId("policy-rule").getByText("已修改", { exact: true })).toHaveCount(0);
      await editRuleId(page, "nyc-to-sha", "nyc-to-sha-edited");
      await selectNode(page, "preview-source", /nyc/);
      await selectNode(page, "preview-target", /sha/);
      await page.getByRole("button", { name: "编译预览", exact: true }).click();
      await expect(page.getByTestId("route-preview")).toContainText("lax");
      await selectNode(page, "preview-target", /^外部目标（域名 \/ IP）$/);
      await page.getByLabel("域名", { exact: true }).fill("allowed.example.cn");
      await page.getByRole("button", { name: "编译预览", exact: true }).click();
      await expect(page.getByTestId("route-preview")).toContainText("allowed.example.cn");
      await page.getByLabel("域名", { exact: true }).fill("denied.invalid");
      await expect(page.getByTestId("route-preview")).toContainText("allowed.example.cn");
      await expect(page.getByTestId("route-preview")).not.toContainText("denied.invalid");
      expect((await getPolicy(controllerUrl, admin)).policy.rules[0]?.id).toBe("nyc-to-sha");
      const mutation = page.waitForRequest((request) =>
        request.method() === "POST" && new URL(request.url()).pathname === "/routing",
      );
      await page.getByRole("button", { name: "保存策略", exact: true }).click();
      const request = await mutation;
      await expect.poll(async () => (await getPolicy(controllerUrl, admin)).policy.rules[0]?.id)
        .toBe("nyc-to-sha-edited");
      await expect(page.getByTestId("policy-save-status")).toContainText("已保存");
      await expect(page.getByRole("button", { name: "保存策略", exact: true })).toBeDisabled();
      await expect(page.getByTestId("policy-rule").getByText("已修改", { exact: true })).toHaveCount(0);
      await expect(page.getByTestId("policy-editor")).not.toContainText("保存前需要载入最新版本");
      const headers = Object.fromEntries(Object.entries(await request.allHeaders()).filter(([name]) =>
        ["content-type", "next-action", "next-router-state-tree"].includes(name.toLowerCase()),
      ));
      const body = request.postData();
      if (!body) throw new Error("Expected policy save action body");
      const response = await anonymous.request.post(request.url(), {
        headers: { ...headers, origin: webUrl }, data: body, maxRedirects: 0,
      });
      expect(response.headers()["x-action-redirect"] ?? response.headers().location ?? "").toContain("/login");
      expect(await page.content()).not.toContain(admin);
    });

    await test.step("a completed save expires deletion undo before rule keys can be reused", async () => {
      const original = await getPolicy(controllerUrl, admin);
      await page.getByRole("button", { name: "规则 nyc-to-sha-edited 的操作", exact: true }).click();
      await page.getByRole("menuitem", { name: "删除", exact: true }).click();
      await expect(page.getByRole("button", { name: "撤销", exact: true })).toBeVisible();
      await page.getByRole("button", { name: "保存策略", exact: true }).click();
      await expect.poll(async () => (await getPolicy(controllerUrl, admin)).policy.rules.length).toBe(1);
      await expect(page.getByTestId("policy-save-status")).toContainText("已保存");
      await expect(page.getByRole("button", { name: "撤销", exact: true })).not.toBeVisible();
      const saved = await getPolicy(controllerUrl, admin);
      expect(saved.policy.rules[0]?.id).toBe("us-to-cn");
      await savePolicy(controllerUrl, admin, { expected_revision: saved.revision, policy: original.policy });
    });

    await test.step("Cmd-K, history and cancelled reload preserve the draft and its original CAS base", async () => {
      // Reload once so the editor is unambiguously based on the successful save.
      await page.goto(`${webUrl}/routing`);
      const latest = await getPolicy(controllerUrl, admin);
      const baseLabel = await page.getByTestId("policy-base-revision").innerText();
      await editRuleId(page, "nyc-to-sha-edited", "draft-survives-conflict");
      await page.keyboard.press("ControlOrMeta+k");
      await page.getByRole("option", { name: "节点", exact: true }).click();
      await expect(page).toHaveURL(`${webUrl}/nodes`);

      const dialogPromise = page.waitForEvent("dialog");
      const reload = page.reload({ timeout: 10_000 }).catch(() => null);
      const unload = await dialogPromise;
      expect(unload.type()).toBe("beforeunload");
      await unload.dismiss();
      await reload;
      await expect(page).toHaveURL(`${webUrl}/nodes`);

      await savePolicy(controllerUrl, admin, {
        expected_revision: latest.revision,
        policy: { ...latest.policy, default_action: "direct" },
      });
      await page.goBack();
      await expect(page).toHaveURL(`${webUrl}/routing`);
      await expect(page.getByTestId("policy-rule").filter({ hasText: "draft-survives-conflict" })).toBeVisible();
      await expect(page.getByTestId("policy-base-revision")).toHaveText(baseLabel);
      await page.getByRole("button", { name: "保存策略", exact: true }).click();
      await expect(page.getByTestId("policy-editor")).toContainText(/409|冲突|版本.*(变化|更新)/);
      await expect(page.getByTestId("policy-rule").filter({ hasText: "draft-survives-conflict" })).toBeVisible();
      expect((await getPolicy(controllerUrl, admin)).policy.rules[0]?.id).toBe("nyc-to-sha-edited");
      await discardPolicyDraft(page);
    });

    const state = join(temporary, "nyc.json");
    const cache = `${state}.config.json`;
    await test.step("agent validation and durable caching precede the controller receipt", async () => {
      const policySnapshot = await getPolicy(controllerUrl, admin);
      await run(executable("meshway-agent"), ["sync", "--state", state], { timeout: 15_000 });
      const node = named((await listNodes(controllerUrl, admin)).nodes, "nyc");
      expect(node.acknowledged_revision).toBe(policySnapshot.revision);
      expect(node.configuration_revision).toBe(policySnapshot.revision);
      const before = await stat(cache);
      expect(before.mode & 0o777).toBe(0o600);
      const stored = await readFile(cache, "utf8");
      expect(stored).toContain("nyc-to-sha-edited");
      await run(executable("meshway-agent"), ["heartbeat", "--state", state], { timeout: 15_000 });
      await run(executable("meshway-agent"), ["sync", "--state", state], { timeout: 15_000 });
      expect((await getPolicy(controllerUrl, admin)).revision).toBe(policySnapshot.revision);
      expect((await stat(cache)).mtimeMs).toBe(before.mtimeMs);
    });

    await test.step("restart preserves addresses, policy and acknowledged revision", async () => {
      const before = await getPolicy(controllerUrl, admin);
      const addresses = (await listNodes(controllerUrl, admin)).nodes.map((node) => [node.id, node.mesh_ipv4, node.mesh_ipv6]);
      await controller.stop();
      controller = startController();
      await controller.ready(`${controllerUrl}/api/v1/overview`, 401);
      expect(await getPolicy(controllerUrl, admin)).toEqual(before);
      expect((await listNodes(controllerUrl, admin)).nodes.map((node) => [node.id, node.mesh_ipv4, node.mesh_ipv6])).toEqual(addresses);
      expect(named((await listNodes(controllerUrl, admin)).nodes, "nyc").acknowledged_revision).toBe(before.revision);
    });

    await test.step("revoking the mandatory hop publishes deny-all rather than a direct fallback", async () => {
      const before = await getPolicy(controllerUrl, admin);
      await revokeNode(controllerUrl, admin, { node_id: lax.id });
      const invalid = await getPolicy(controllerUrl, admin);
      expect(invalid.revision).toBeGreaterThan(before.revision);
      expect(invalid.valid).toBe(false);
      expect(invalid.policy.rules[0]?.action.via).toEqual([lax.id]);
      await run(executable("meshway-agent"), ["sync", "--state", state], { timeout: 15_000 });
      expect(named((await listNodes(controllerUrl, admin)).nodes, "nyc").acknowledged_revision).toBe(invalid.revision);
      const cached: unknown = JSON.parse(await readFile(cache, "utf8"));
      const stored = JSON.stringify(cached);
      expect(stored).toContain('"policy_valid":false');
      expect(stored).toContain('"default_action":"reject"');
      await page.goto(`${webUrl}/routing`);
      await expect(page.getByTestId("policy-editor")).toContainText(/无效|不可用|错误|失效/);
      await page.setViewportSize({ width: 390, height: 844 });
      expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
      expect(browserErrors).toEqual([]);
    });
  } finally {
    await anonymous.close();
    await web?.stop();
    await controller.stop();
    await rm(temporary, { recursive: true, force: true });
  }
});
