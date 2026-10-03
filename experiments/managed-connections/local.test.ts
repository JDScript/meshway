import assert from "node:assert/strict";
import { closeSync, fstatSync, openSync } from "node:fs";
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { command } from "./local.ts";

test("subprocess retains the same private performance lock descriptor and rejects a substituted file", async () => {
  const directory = await mkdtemp(join(tmpdir(), "meshway-lock-child-"));
  const path = join(directory, "lock"), other = join(directory, "other");
  await chmod(directory, 0o700);
  await writeFile(path, "", { mode: 0o600 });
  await writeFile(other, "", { mode: 0o600 });
  const descriptor = openSync(path, "r+");
  const previous = { fd: process.env.MESHWAY_PERF_LOCK_FD, path: process.env.MESHWAY_PERF_LOCK };
  try {
    process.env.MESHWAY_PERF_LOCK_FD = String(descriptor);
    process.env.MESHWAY_PERF_LOCK = path;
    const output = await command(process.execPath, ["-e", "const fs=require('node:fs');const s=fs.fstatSync(Number(process.env.MESHWAY_PERF_LOCK_FD));console.log(JSON.stringify({dev:s.dev,ino:s.ino}));"]);
    const held = fstatSync(descriptor);
    assert.deepEqual(JSON.parse(output), { dev: held.dev, ino: held.ino });
    process.env.MESHWAY_PERF_LOCK = other;
    await assert.rejects(command(process.execPath, ["-e", "process.exit(0)"]), /ownership differs/);
  } finally {
    if (previous.fd === undefined) delete process.env.MESHWAY_PERF_LOCK_FD;
    else process.env.MESHWAY_PERF_LOCK_FD = previous.fd;
    if (previous.path === undefined) delete process.env.MESHWAY_PERF_LOCK;
    else process.env.MESHWAY_PERF_LOCK = previous.path;
    closeSync(descriptor);
    await rm(directory, { recursive: true, force: true });
  }
});
