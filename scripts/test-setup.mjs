// Exercise setup without creating or replacing the repository's actual .env.
import assert from "node:assert/strict";
import { mkdtemp, mkdir, copyFile, readFile, stat } from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
assert.equal(
  process.env.COMPMASTER_DOCKER_TESTS,
  "1",
  "Explicit Docker test opt-in required",
);
const root = await mkdtemp(path.join(process.cwd(), ".cm-image-setup-"));
await mkdir(path.join(root, "scripts"));
await mkdir(path.join(root, "projects"));
await copyFile("scripts/setup.sh", path.join(root, "scripts/setup.sh"));
await copyFile("scripts/compose.sh", path.join(root, "scripts/compose.sh"));
const cli = process.env.DOCKER_CLI || "docker";
const env = {
  ...process.env,
  PATH: cli.includes("/")
    ? `${path.dirname(cli)}:${process.env.PATH}`
    : process.env.PATH,
};
const args = [path.join(root, "scripts/setup.sh"), path.join(root, "projects")];
const first = spawnSync("bash", args, { env, encoding: "utf8" });
assert.equal(first.status, 0, first.stderr);
const content = await readFile(path.join(root, ".env"), "utf8");
assert.match(content, /DOCKER_GID=\d+/);
assert.match(content, new RegExp(`COMPMASTER_UID=${process.getuid()}`));
assert.equal((await stat(path.join(root, ".env"))).mode & 0o777, 0o600);
const config = spawnSync(
  cli,
  [
    "compose",
    "--env-file",
    path.join(root, ".env"),
    "-f",
    path.resolve("compose.yaml"),
    "config",
    "--quiet",
  ],
  { env, encoding: "utf8" },
);
assert.equal(config.status, 0, config.stderr);
const second = spawnSync("bash", args, { env, encoding: "utf8" });
assert.notEqual(second.status, 0);
assert.equal(await readFile(path.join(root, ".env"), "utf8"), content);
console.log(
  "Setup checks passed: non-root identity, socket GID, available port, private .env, Compose validation and no overwrite.",
);
console.log(`Setup test files retained: ${root}`);
