import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const advisory = "https://github.com/advisories/GHSA-fc5q-39xc-vjpf";
const expires = Date.parse("2026-12-18T00:00:00Z");

// Follow npm's derived findings to their actual advisory. Only development
// nodes covered exclusively by this one time-limited exception are accepted.
export function accepted(
  name,
  vulnerabilities,
  packages,
  now = Date.now(),
  seen = new Set(),
) {
  const finding = vulnerabilities[name];
  if (
    now >= expires ||
    seen.has(name) ||
    !finding?.nodes?.length ||
    !finding.via?.length
  )
    return false;
  if (!finding.nodes.every((node) => packages[node]?.dev === true))
    return false;
  const visited = new Set([...seen, name]);
  return finding.via.every((via) =>
    typeof via === "string"
      ? accepted(via, vulnerabilities, packages, now, visited)
      : via.url === advisory &&
        via.name === "picomatch" &&
        via.severity === "high",
  );
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const result = spawnSync("npm", ["--prefix", "frontend", "audit", "--json"], {
    encoding: "utf8",
  });
  assert.ok(
    result.status === 0 || result.status === 1,
    result.stderr || "npm audit failed",
  );
  const report = JSON.parse(result.stdout);
  assert.equal(report.auditReportVersion, 2, "Unsupported npm audit report");
  assert.ok(
    !report.error && report.vulnerabilities,
    "Incomplete npm audit report",
  );
  const { packages } = JSON.parse(
    readFileSync("frontend/package-lock.json", "utf8"),
  );
  const blocked = [];
  for (const [name, finding] of Object.entries(report.vulnerabilities)) {
    if (!["high", "critical"].includes(finding.severity)) continue;
    if (accepted(name, report.vulnerabilities, packages)) {
      console.warn(
        `Temporary development-only exception: ${name}, ${advisory}, expires 2026-12-18`,
      );
    } else {
      blocked.push({ name, ...finding });
    }
  }
  assert.equal(blocked.length, 0, JSON.stringify(blocked, null, 2));
  console.log(
    "Frontend audit passed; see docs/SECURITY.md for the scoped development exception.",
  );
}
