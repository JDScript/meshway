import { readdirSync, existsSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const root = fileURLToPath(new URL("../", import.meta.url));
const experiments = join(root, "experiments");
const projects = readdirSync(experiments, { withFileTypes: true })
  .filter((entry) => entry.isDirectory() && existsSync(join(experiments, entry.name, "tsconfig.json")))
  .map((entry) => entry.name)
  .sort();

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

if (projects.length === 0) throw new Error("No experiment TypeScript projects found");
for (const project of projects) {
  console.log(`Checking experiments/${project}`);
  run("pnpm", ["exec", "tsc", "--noEmit", "-p", `experiments/${project}/tsconfig.json`]);
  const tests = readdirSync(join(experiments, project))
    .filter((name) => name.endsWith(".test.ts"))
    .sort()
    .map((name) => `experiments/${project}/${name}`);
  if (tests.length) run(process.execPath, ["--test", ...tests]);
}
