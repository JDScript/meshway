import { spawnSync } from "node:child_process";
import { mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const artifactPaths = [
  "packages/protocol/openapi.json",
  "apps/agent/src/generated",
  "packages/control-client/src/generated",
];

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status})`);
}

export async function generate(outputRoot = root) {
  const schema = join(outputRoot, artifactPaths[0]);
  const rust = join(outputRoot, artifactPaths[1]);
  const typescript = join(outputRoot, artifactPaths[2]);
  await mkdir(dirname(schema), { recursive: true });
  run("cargo", ["run", "--locked", "--quiet", "-p", "meshway-controller", "--", "export-openapi", "--output", schema]);
  run("cargo", ["run", "--locked", "--quiet", "-p", "meshway-controller", "--features", "codegen", "--bin", "generate-rust-client", "--", "--input", schema, "--output", rust]);
  const rustFiles = (await readdir(rust)).filter((file) => file.endsWith(".rs"));
  run("rustfmt", ["--edition", "2024", ...rustFiles.map((file) => join(rust, file))]);

  const require = createRequire(join(root, "packages/control-client/package.json"));
  const generatorPackage = dirname(require.resolve("@hey-api/openapi-ts/package.json"));
  const { createClient } = await import(pathToFileURL(join(generatorPackage, "dist/index.mjs")).href);
  await rm(typescript, { recursive: true, force: true });
  await createClient({
    configFile: false,
    input: schema,
    output: { path: typescript },
    plugins: [
      "@hey-api/typescript",
      { name: "zod", dates: { offset: true } },
    ],
  });
  await writeFile(join(typescript, "operations.gen.ts"), operations(JSON.parse(await readFile(schema, "utf8"))));
}

// Hey API emits the DTOs and Zod schemas. This deliberately narrow operation
// emitter keeps the transport free of unchecked generic casts. It fails closed
// when the API grows beyond JSON GET/POST operations with component schemas.
function operations(document) {
  if (document.openapi !== "3.1.0") throw new Error("Expected native OpenAPI 3.1.0");
  const types = new Set();
  const validators = new Set();
  const names = new Set();
  const functions = [];
  function component(content) {
    if (Object.keys(content).join() !== "application/json") throw new Error("Only JSON media is supported by the TypeScript operation emitter");
    const schema = content["application/json"].schema;
    const match = /^#\/components\/schemas\/([A-Za-z][A-Za-z0-9]*)$/.exec(schema.$ref);
    if (!match || Object.keys(schema).length !== 1) throw new Error("Operations must reference component schemas");
    const name = match[1];
    if (!document.components.schemas[name]) throw new Error(`Missing component ${name}`);
    types.add(name);
    validators.add(`z${name}`);
    return name;
  }
  for (const [path, item] of Object.entries(document.paths)) {
    if (!/^\/[a-zA-Z0-9/_-]+$/.test(path)) throw new Error(`Unsupported operation path: ${path}`);
    for (const [method, operation] of Object.entries(item)) {
      if (!["get", "post"].includes(method) || operation.parameters?.length || operation.security?.length || document.security?.length) {
        throw new Error(`Unsupported operation shape: ${method} ${path}; extend the emitter first`);
      }
      const name = operation.operationId;
      if (!/^[a-z][A-Za-z0-9]+$/.test(name) || names.has(name)) throw new Error(`Invalid or duplicate operationId: ${name}`);
      names.add(name);
      const successes = Object.keys(operation.responses).filter((status) => /^2\d\d$/.test(status));
      if (successes.length !== 1 || successes[0] !== "200") throw new Error("Exactly one HTTP 200 response is currently supported");
      const response = component(operation.responses["200"].content);
      let body;
      if (operation.requestBody) {
        if (!operation.requestBody.required || method !== "post") throw new Error("Only required POST request bodies are currently supported");
        body = component(operation.requestBody.content);
      }
      functions.push(`export async function ${name}Operation(baseUrl: string${body ? `, body: ${body}` : ""}): Promise<${response}> {\n  return request(baseUrl, { method: ${JSON.stringify(method.toUpperCase())}, path: ${JSON.stringify(path)}${body ? `, body: z${body}.parse(body)` : ""} }, z${response});\n}`);
    }
  }
  return `// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.\n\nimport { request } from "../transport.ts";\nimport type { ${[...types].sort().join(", ")} } from "./types.gen.ts";\nimport { ${[...validators].sort().join(", ")} } from "./zod.gen.ts";\n\n${functions.join("\n\n")}\n`;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await generate();
}
