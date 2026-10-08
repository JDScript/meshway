import { mkdtemp, readdir, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { artifactPaths, generate, root } from "./generate.mjs";

async function files(path, prefix = "") {
  if (!(await stat(path)).isDirectory()) return [prefix];
  const nested = await Promise.all((await readdir(path)).sort().map(async (name) =>
    files(join(path, name), join(prefix, name)),
  ));
  return nested.flat();
}

export async function compareArtifacts(expectedDirectory, actualDirectory, artifacts = artifactPaths) {
  const stale = [];
  for (const artifact of artifacts) {
    const expectedRoot = join(expectedDirectory, artifact);
    const actualRoot = join(actualDirectory, artifact);
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
  return stale.sort();
}

export async function check() {
  const temporary = await mkdtemp(join(tmpdir(), "meshway-protocol-"));
  try {
    await generate(temporary);
    const stale = await compareArtifacts(temporary, root);
    if (stale.length) {
    throw new Error(`Protocol artifacts are stale:\n${stale.join("\n")}\nRun mise run generate:protocol`);
    }
    console.log("OpenAPI, Rust operations, TypeScript operations and validators are current.");
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}
if (process.argv[1] === fileURLToPath(import.meta.url)) await check();
