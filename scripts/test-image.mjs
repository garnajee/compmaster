// Verify the built image under its production restrictions. Exact cm-test-* resources only.
import assert from "node:assert/strict";
import { mkdtemp, writeFile, chmod, readFile } from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { createServer } from "node:net";
const cli = process.env.DOCKER_CLI || "docker";
const image = process.argv[2] || "compmaster:dev";
const command = (...args) => {
  const output = spawnSync(cli, args, { encoding: "utf8", env: process.env });
  assert.equal(output.status, 0, output.stderr || output.error?.message);
  return output.stdout.trim();
};
const name = `cm-test-image-${Date.now().toString(36)}`;
const root = await mkdtemp(path.join(process.cwd(), ".cm-image-"));
await chmod(root, 0o777);
const file = path.join(root, "custom.yaml");
await writeFile(
  file,
  `name: ${name}-project\nservices:\n  app:\n    image: ${name}-service:local\n    build: .\n`,
);
await writeFile(
  path.join(root, "Dockerfile"),
  'FROM alpine:latest\nCMD ["sleep", "120"]\n',
);
const socket =
  process.env.COMPMASTER_TEST_DAEMON_SOCKET || "/var/run/docker.sock";
const identity = process.env.COMPMASTER_TEST_UID
  ? `${process.env.COMPMASTER_TEST_UID}:${process.env.COMPMASTER_TEST_GID || process.env.COMPMASTER_TEST_UID}`
  : "65532:65532";
const port = await new Promise((resolve, reject) => {
  const listener = createServer();
  listener.once("error", reject);
  listener.listen(0, "127.0.0.1", () => {
    const port = listener.address().port;
    listener.close(() => resolve(port));
  });
});
const base = `http://127.0.0.1:${port}`;
const gid = command(
  "run",
  "--rm",
  "--network",
  "none",
  "--read-only",
  "--cap-drop",
  "ALL",
  "--security-opt",
  "no-new-privileges",
  "-v",
  `${socket}:/socket:ro`,
  "alpine:latest",
  "stat",
  "-c",
  "%g",
  "/socket",
);
try {
  command(
    "run",
    "-d",
    "--name",
    name,
    "--user",
    identity,
    "--read-only",
    "--cap-drop",
    "ALL",
    "--security-opt",
    "no-new-privileges",
    "--group-add",
    gid,
    "--group-add",
    "65532",
    "--tmpfs",
    "/tmp:rw,noexec,nosuid,size=128m,mode=1777",
    "--mount",
    `type=bind,src=${root},dst=${root}`,
    "--mount",
    `type=bind,src=${socket},dst=/var/run/docker.sock`,
    "--mount",
    `type=volume,src=${name}-data,dst=/data`,
    "-e",
    `COMPOSE_ROOT=${root}`,
    "-e",
    `COMPMASTER_PUBLIC_ORIGIN=${base}`,
    "-p",
    `127.0.0.1:${port}:8080`,
    image,
  );
  const headers = {};
  let session;
  for (let i = 0; i < 100; i++) {
    try {
      const r = await fetch(`${base}/api/v1/session`, { headers });
      if (r.ok) {
        session = await r.json();
        break;
      }
    } catch {}
    await new Promise((r) => setTimeout(r, 300));
  }
  assert.ok(session, "Image did not become ready");
  headers["X-CSRF-Token"] = session.csrfToken;
  headers["Content-Type"] = "application/json";
  const inv = await fetch(`${base}/api/v1/inventory`, { headers }).then((r) =>
    r.json(),
  );
  assert.equal(inv.engineConnected, true, inv.engineError);
  const project = inv.projects.find((p) => p.name === `${name}-project`);
  assert.ok(
    project?.definition?.valid,
    "Bundled Docker Compose must validate YAML",
  );
  const source = await fetch(`${base}/api/v1/projects/${project.id}/file`, {
    headers,
  }).then((r) => r.json());
  const saved = await fetch(`${base}/api/v1/projects/${project.id}/file`, {
    method: "PUT",
    headers,
    body: JSON.stringify({
      content: source.content + "# image write test\n",
      revision: source.revision,
    }),
  });
  assert.equal(saved.status, 200, await saved.text());
  assert.match(await readFile(file, "utf8"), /image write test/);
  const runJob = async (action, options = {}) => {
    const response = await fetch(`${base}/api/v1/jobs`, {
      method: "POST",
      headers,
      body: JSON.stringify({ target: project.id, action, options }),
    });
    assert.equal(response.status, 200);
    const queued = await response.json();
    for (let i = 0; i < 200; i++) {
      const jobs = await fetch(`${base}/api/v1/jobs`, { headers }).then((r) =>
        r.json(),
      );
      const job = jobs.find((j) => j.id === queued.id);
      if (["succeeded", "failed"].includes(job.state)) {
        assert.equal(job.state, "succeeded", job.error);
        return;
      }
      await new Promise((r) => setTimeout(r, 400));
    }
    throw new Error(`Image job timed out: ${action}`);
  };
  await runJob("up_build");
  await runJob("down", { confirmed: true });
  command("exec", name, "/usr/local/bin/compmaster", "--healthcheck");
  command("exec", name, "/usr/local/bin/docker", "compose", "version");
  command("exec", name, "/usr/local/bin/docker", "buildx", "version");
  const inspect = JSON.parse(command("inspect", name))[0];
  assert.equal(inspect.Config.User, identity);
  assert.equal(inspect.HostConfig.ReadonlyRootfs, true);
  assert.deepEqual(inspect.HostConfig.CapDrop, ["ALL"]);
  const shell = spawnSync(cli, ["exec", name, "/bin/sh", "-c", "id"], {
    env: process.env,
    encoding: "utf8",
  });
  assert.notEqual(shell.status, 0, "Production image must have no shell");
  console.log(
    "Image checks passed: non-root, read-only rootfs, no shell, socket by GID, Compose/Buildx build/up/down, healthcheck and atomic YAML write.",
  );
} catch (error) {
  const logs = spawnSync(cli, ["logs", "--tail", "30", name], {
    env: process.env,
    encoding: "utf8",
  });
  console.error(logs.stdout, logs.stderr);
  const status = spawnSync(
    cli,
    ["inspect", "--format", "{{json .State}}", name],
    { env: process.env, encoding: "utf8" },
  );
  console.error(status.stdout);
  throw error;
} finally {
  spawnSync(cli, ["compose", "-p", `${name}-project`, "-f", file, "down"], {
    env: process.env,
    stdio: "ignore",
  });
  spawnSync(cli, ["image", "rm", `${name}-service:local`], {
    env: process.env,
    stdio: "ignore",
  });
  spawnSync(cli, ["container", "rm", "-f", name], {
    env: process.env,
    stdio: "ignore",
  });
  spawnSync(cli, ["volume", "rm", `${name}-data`], {
    env: process.env,
    stdio: "ignore",
  });
  console.log(`Image test files retained: ${root}`);
}
