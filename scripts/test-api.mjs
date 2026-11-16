// End-to-end tests against a real local Docker engine. Only cm-test-* resources are mutated.
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, readFile, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:net";
import { createRequire } from "node:module";

assert.equal(
  process.env.COMPMASTER_DOCKER_TESTS,
  "1",
  "Set COMPMASTER_DOCKER_TESTS=1 to opt in",
);
assert.ok(
  process.env.DOCKER_HOST?.startsWith("unix://"),
  "Set DOCKER_HOST to the local engine explicitly",
);
const root = await mkdtemp(path.join(tmpdir(), "cm-test-api-"));
const projects = path.join(root, "projects");
await mkdir(projects);
const prefix = `cm-test-${Date.now().toString(36)}`;
const cli = process.env.DOCKER_CLI || "docker";
const command = (...args) => {
  const output = spawnSync(cli, args, { encoding: "utf8", env: process.env });
  assert.equal(output.status, 0, output.stderr || output.error?.message);
  return output.stdout.trim();
};
const reserve = () =>
  new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const port = server.address().port;
      server.close(() => resolve(port));
    });
  });
const webPort = await reserve();
const apiPort = await reserve();
const base = `http://127.0.0.1:${apiPort}`;
const file = path.join(projects, "arbitrary-name.yaml");
const secondFile = path.join(projects, "another-name.yaml");
const multiBase = path.join(projects, "multi-base.yaml");
const multiOverride = path.join(projects, "multi-extra.yaml");
const yaml = `name: ${prefix}-compose\nservices:\n  web:\n    image: alpine:latest\n    command: ["sh", "-c", "echo cm-ready; sleep 300"]\n    ports:\n      - "127.0.0.1:${webPort}:8000"\n`;
await writeFile(
  path.join(projects, ".env"),
  `CM_TEST_PROJECT_NAME=${prefix}-compose\n`,
);
await writeFile(
  file,
  yaml.replace(`${prefix}-compose`, "${CM_TEST_PROJECT_NAME}"),
);
await writeFile(
  secondFile,
  yaml.replace(`${prefix}-compose`, `${prefix}-conflict`),
);
await writeFile(
  multiBase,
  `services:\n  worker:\n    image: alpine:latest\n    command: ["sleep", "300"]\n`,
);
await writeFile(
  multiOverride,
  'services:\n  worker:\n    environment:\n      CM_MULTI: "yes"\n',
);
const server = spawn(path.resolve("target/debug/compmaster"), [], {
  env: {
    ...process.env,
    COMPOSE_ROOT: projects,
    COMPMASTER_DATA_DIR: path.join(root, "data"),
    COMPMASTER_LISTEN: `127.0.0.1:${apiPort}`,
    COMPMASTER_PUBLIC_ORIGIN: base,
    COMPMASTER_PROBE_HOST: "127.0.0.1",
  },
  stdio: ["ignore", "pipe", "pipe"],
});
let serverLog = "";
server.stdout.on("data", (data) => {
  serverLog += data;
});
server.stderr.on("data", (data) => {
  serverLog += data;
});
let token = "";
const api = async (url, method = "GET", body, expected = 200) => {
  const response = await fetch(base + "/api/v1" + url, {
    method,
    headers: {
      "Content-Type": "application/json",
      Origin: base,
      "X-CSRF-Token": token,
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const value = await response.json();
  assert.equal(response.status, expected, JSON.stringify(value));
  return value;
};
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const job = async (
  target,
  action,
  options = {},
  expectedState = "succeeded",
) => {
  const queued = await api("/jobs", "POST", { target, action, options });
  for (let attempt = 0; attempt < 150; attempt++) {
    const current = (await api("/jobs")).find((j) => j.id === queued.id);
    if (["succeeded", "failed"].includes(current.state)) {
      assert.equal(current.state, expectedState, current.error);
      return current;
    }
    await sleep(400);
  }
  throw new Error(`Job timed out: ${action}`);
};
const inspect = (id) => JSON.parse(command("container", "inspect", id))[0];
const waitJob = async (queued) => {
  for (let i = 0; i < 100; i++) {
    const current = (await api("/jobs")).find((j) => j.id === queued.id);
    if (["succeeded", "failed"].includes(current.state)) {
      assert.equal(current.state, "succeeded", current.error);
      return current;
    }
    await sleep(300);
  }
  throw new Error("Job timed out");
};
const standaloneName = `${prefix}-standalone`;
let passed = 0;
try {
  for (let i = 0; i < 100; i++) {
    try {
      const session = await api("/session");
      token = session.csrfToken;
      break;
    } catch {
      await sleep(200);
    }
  }
  assert.ok(token, serverLog);
  let inventory = await api("/inventory");
  const project = inventory.projects.find(
    (p) => p.name === `${prefix}-compose`,
  );
  const conflict = inventory.projects.find(
    (p) => p.name === `${prefix}-conflict`,
  );
  assert.ok(project?.definition?.valid);
  assert.ok(conflict?.definition?.valid);
  passed++;
  await job(project.id, "up");
  await api("/refresh", "POST");
  inventory = await api("/inventory");
  const launched = inventory.projects.find((p) => p.name === project.name);
  assert.equal(launched.id, project.id, "Project identity must survive up");
  assert.equal(launched.containers.length, 1);
  assert.ok(launched.containers[0].networkAddresses);
  passed++;
  const blocked = await job(conflict.id, "up", {}, "failed");
  assert.match(blocked.error, /[Pp]ort/);
  passed++;

  const source = await api(`/projects/${encodeURIComponent(project.id)}/file`);
  await api(
    `/projects/${encodeURIComponent(project.id)}/file`,
    "PUT",
    { content: yaml, revision: "stale" },
    409,
  );
  await api(
    `/projects/${encodeURIComponent(project.id)}/validate`,
    "POST",
    { content: "services: [broken", revision: source.revision },
    400,
  );
  await api(`/projects/${encodeURIComponent(project.id)}/file`, "PUT", {
    content: yaml + "# saved through API\n",
    revision: source.revision,
  });
  assert.match(await readFile(file, "utf8"), /saved through API/);
  assert.equal((await readdir(path.join(root, "data", "backups"))).length, 1);
  passed++;

  const container = launched.containers[0];
  const logAbort = new AbortController();
  const logResponse = await fetch(
    `${base}/api/v1/containers/${container.id}/logs`,
    { signal: logAbort.signal },
  );
  const reader = logResponse.body.getReader();
  let logs = "";
  for (let i = 0; i < 10 && !logs.includes("cm-ready"); i++) {
    const value = await reader.read();
    logs += new TextDecoder().decode(value.value);
  }
  logAbort.abort();
  assert.match(logs, /cm-ready/);
  passed++;
  const require = createRequire(path.resolve("frontend/package.json"));
  const WebSocket = require("ws");
  await new Promise((resolve, reject) => {
    const ws = new WebSocket(
      `${base.replace("http:", "ws:")}/ws/terminal/${container.id}?token=${token}&command=%2Fbin%2Fsh`,
      { headers: { Origin: base } },
    );
    const timeout = setTimeout(() => {
      ws.close();
      reject(new Error("Terminal timeout"));
    }, 10000);
    let output = "";
    ws.on("open", () =>
      ws.send(Buffer.from('printf "cm-terminal-%s\\n" ok\n')),
    );
    ws.on("error", reject);
    ws.on("message", (data) => {
      output += data.toString();
      if (output.includes("cm-terminal-ok")) {
        clearTimeout(timeout);
        ws.close();
        resolve();
      }
    });
  });
  passed++;

  const oldId = command(
    "run",
    "-d",
    "--name",
    standaloneName,
    "alpine:latest",
    "sleep",
    "300",
  );
  await api("/refresh", "POST");
  await job(oldId, "replace", {
    confirmed: true,
    config: { Cmd: ["sh", "-c", "echo replacement; sleep 300"] },
  });
  const replacement = inspect(standaloneName);
  assert.notEqual(replacement.Id, oldId);
  assert.equal(replacement.State.Running, true);
  assert.match(inspect(oldId).Name, /cm-backup/);
  await job(
    replacement.Id,
    "replace",
    { confirmed: true, config: { Cmd: ["/nonexistent-cm-command"] } },
    "failed",
  );
  assert.equal(inspect(standaloneName).Id, replacement.Id);
  assert.equal(inspect(standaloneName).State.Running, true);
  passed++;

  const network = `${prefix}-network`,
    volume = `${prefix}-volume`;
  await job("network:new", "create_network", {
    name: network,
    driver: "bridge",
  });
  await job("volume:new", "create_volume", { name: volume, driver: "local" });
  inventory = await api("/inventory");
  const networkId = inventory.networks.find((n) => n.name === network).id;
  await job(`network:${networkId}`, "connect", {
    confirmed: true,
    container: replacement.Id,
  });
  await job(`network:${networkId}`, "disconnect", {
    confirmed: true,
    container: replacement.Id,
  });
  await job(`network:${networkId}`, "remove", { confirmed: true });
  await job(`volume:${volume}`, "remove", { confirmed: true });
  passed++;

  const preview = await api("/prune/preview");
  assert.ok(
    !preview.some((r) => r.kind === "container" && oldId.startsWith(r.id)),
    "Backup must be excluded from prune",
  );
  passed++;
  const pruneId = command(
    "create",
    "--name",
    `${prefix}-prune`,
    "alpine:latest",
    "true",
  );
  // Created containers are deliberately not proposed for prune until they have exited.
  command("start", "-a", pruneId);
  const selected = (await api("/prune/preview")).find(
    (r) => r.kind === "container" && pruneId.startsWith(r.id),
  );
  assert.ok(selected);
  await waitJob(
    await api("/prune", "POST", { confirmed: true, resources: [selected] }),
  );
  passed++;

  command(
    "compose",
    "-p",
    `${prefix}-multi`,
    "-f",
    multiBase,
    "-f",
    multiOverride,
    "up",
    "-d",
  );
  await api("/refresh", "POST");
  inventory = await api("/inventory");
  const multi = inventory.projects.find((p) => p.name === `${prefix}-multi`);
  assert.equal(multi.definition.files.length, 2);
  const extra = await api(
    `/projects/${encodeURIComponent(multi.id)}/file?index=1`,
  );
  assert.match(extra.content, /CM_MULTI/);
  await api(`/projects/${encodeURIComponent(multi.id)}/file?index=1`, "PUT", {
    content: extra.content + "# validated in combined config\n",
    revision: extra.revision,
  });
  await job(multi.id, "restart", { confirmed: true });
  await job(multi.id, "down", { confirmed: true });
  passed++;
  await job(project.id, "down", { confirmed: true });
  console.log(
    `${passed} API integration scenarios passed (discovery, labels, conflicts, editor, logs, terminal, replacement/rollback, resources, backup protection).`,
  );
} finally {
  // Exact project and container names only. Never prune unrelated engine resources.
  for (const [name, source] of [
    [`${prefix}-compose`, file],
    [`${prefix}-conflict`, secondFile],
    [`${prefix}-multi`, multiBase],
  ])
    spawnSync(cli, ["compose", "-p", name, "-f", source, "down"], {
      env: process.env,
      stdio: "ignore",
    });
  const ids =
    spawnSync(
      cli,
      ["container", "ls", "-a", "--format", "{{.ID}} {{.Names}}"],
      { env: process.env, encoding: "utf8" },
    ).stdout || "";
  for (const line of ids.trim().split("\n")) {
    const [id, name] = line.split(" ");
    if (name?.startsWith(prefix))
      spawnSync(cli, ["container", "rm", "-f", id], {
        env: process.env,
        stdio: "ignore",
      });
  }
  for (const kind of ["network", "volume"])
    spawnSync(cli, [kind, "rm", `${prefix}-${kind}`], {
      env: process.env,
      stdio: "ignore",
    });
  server.kill("SIGTERM");
  console.log(`Test files retained for inspection: ${root}`);
}
