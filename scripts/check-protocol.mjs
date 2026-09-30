import { mkdtemp, readdir, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { artifactPaths, generate, root } from "./generate-protocol.mjs";

async function files(path, prefix = "") {
  if (!(await stat(path)).isDirectory()) return [prefix];
  const nested = await Promise.all((await readdir(path)).sort().map(async (name) =>
    files(join(path, name), join(prefix, name)),
  ));
  return nested.flat();
}

const temporary = await mkdtemp(join(tmpdir(), "meshway-protocol-"));
try {
  await generate(temporary);
  const stale = [];
  for (const artifact of artifactPaths) {
    const expectedRoot = join(temporary, artifact);
    const actualRoot = join(root, artifact);
    const expected = await files(expectedRoot);
    let actual;
    try { actual = await files(actualRoot); } catch { actual = []; }
    for (const file of new Set([...expected, ...actual])) {
      try {
        const [a, b] = await Promise.all([
          readFile(join(expectedRoot, file)),
          readFile(join(actualRoot, file)),
        ]);
        if (!a.equals(b)) stale.push(join(artifact, file));
      } catch { stale.push(join(artifact, file)); }
    }
  }
  if (stale.length) {
    throw new Error(`Protocol artifacts are stale:\n${stale.join("\n")}\nRun pnpm --filter @meshway/control-client generate`);
  }
  console.log("OpenAPI, Rust operations, TypeScript operations and validators are current.");
} finally {
  await rm(temporary, { recursive: true, force: true });
}
