import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { createServer } from "node:http";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { getOverview, listNodes } from "../src/index.ts";
import { reportAgentOperation } from "../src/generated/operations.gen.ts";

async function withResponse(
  body: string,
  use: (baseUrl: string) => Promise<void>,
  status = 200,
  contentType = "application/json",
): Promise<void> {
  const server = createServer((_request, response) => {
    response.writeHead(status, { "content-type": contentType });
    response.end(body);
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert.ok(address && typeof address !== "string");
  try {
    await use(`http://127.0.0.1:${address.port}`);
  } finally {
    server.closeAllConnections();
    server.close();
    await once(server, "close");
  }
}

test("generated overview validation rejects missing, wrong-type, fractional and negative values", async () => {
  for (const body of [
    { name: "Meshway", version: "0.1.0" },
    { name: "Meshway", version: "0.1.0", node_count: "1" },
    { name: "Meshway", version: "0.1.0", node_count: 1.5 },
    { name: "Meshway", version: "0.1.0", node_count: -1 },
  ]) {
    await withResponse(JSON.stringify(body), async (base) => {
      await assert.rejects(getOverview(base), /Controller GET .*api\/v1\/overview/);
    });
  }
});

test("generated node validation checks nested fields and RFC3339 timestamps", async () => {
  for (const last_seen of ["not-a-date", "2026-09-30T12:00:00", 42]) {
    await withResponse(JSON.stringify({ nodes: [{ id: "n", name: "nyc", agent_version: "0.1.0", last_seen }] }), async (base) => {
      await assert.rejects(listNodes(base), /last_seen/);
    });
  }
  await withResponse(JSON.stringify({ nodes: [{ id: "n", name: "nyc", agent_version: "0.1.0", last_seen: "2026-09-30T12:00:00+01:00" }] }), async (base) => {
    assert.equal((await listNodes(base)).nodes[0]?.id, "n");
  });
});

test("transport exposes informative HTTP, JSON and media-type failures", async () => {
  await withResponse("unavailable", async (base) => {
    await assert.rejects(getOverview(base), /Controller GET .*HTTP 503/);
  }, 503);
  await withResponse("{", async (base) => {
    await assert.rejects(getOverview(base), /Controller GET/);
  });
  await withResponse("{}", async (base) => {
    await assert.rejects(getOverview(base), /application\/json/);
  }, 200, "text/html");
  await withResponse(" ".repeat(2 * 1024 * 1024 + 1), async (base) => {
    await assert.rejects(getOverview(base), /exceeds 2 MiB/);
  });
});

test("generated TS report and web operations roundtrip through a real controller", { timeout: 120_000 }, async () => {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
  const build = spawnSync("cargo", ["build", "--locked", "--quiet", "-p", "meshway-controller"], { cwd: root, encoding: "utf8" });
  assert.equal(build.status, 0, build.stderr);
  const child = spawn(resolve(root, "target/debug/meshway-controller"), ["serve", "--bind", "127.0.0.1:0"], { cwd: root, stdio: ["ignore", "ignore", "pipe"] });
  try {
    const base = await new Promise<string>((accept, reject) => {
      let stderr = "";
      child.once("error", reject);
      child.once("exit", (code) => reject(new Error(`controller exited (${code}): ${stderr}`)));
      child.stderr.on("data", (chunk: Buffer) => {
        stderr += chunk.toString("utf8");
        const match = /http:\/\/127\.0\.0\.1:\d+/.exec(stderr);
        if (match) accept(match[0]);
      });
    });
    assert.equal((await getOverview(base)).node_count, 0);
    assert.deepEqual(await listNodes(base), { nodes: [] });
    const report = await reportAgentOperation(base, { node_id: "ts-roundtrip", name: "nyc", agent_version: "0.1.0" });
    assert.equal(report.node.id, "ts-roundtrip");
    assert.equal((await getOverview(base)).node_count, 1);
    assert.deepEqual((await listNodes(base)).nodes, [report.node]);
    await assert.rejects(reportAgentOperation(base, { node_id: "bad", name: " ", agent_version: "0.1.0" }));
    assert.equal((await getOverview(base)).node_count, 1);
  } finally {
    child.kill("SIGTERM");
    await once(child, "exit");
  }
});
