import assert from "node:assert/strict";
import test from "node:test";
import { accepted } from "./audit-frontend.mjs";

const now = Date.parse("2026-11-18T00:00:00Z");
const via = {
  name: "picomatch",
  severity: "high",
  url: "https://github.com/advisories/GHSA-fc5q-39xc-vjpf",
};
const findings = {
  picomatch: { nodes: ["node_modules/picomatch"], via: [via] },
  vite: { nodes: ["node_modules/vite"], via: ["picomatch"] },
};
const packages = {
  "node_modules/picomatch": { dev: true },
  "node_modules/vite": { dev: true },
};

test("accepts only development findings derived from the reviewed advisory", () => {
  assert.equal(accepted("vite", findings, packages, now), true);
  assert.equal(
    accepted(
      "vite",
      findings,
      { ...packages, "node_modules/picomatch": {} },
      now,
    ),
    false,
  );
  assert.equal(accepted("vite", findings, {}, now), false);
});
test("blocks other advisories and mixed findings", () => {
  const changed = structuredClone(findings);
  changed.picomatch.via.push({
    ...via,
    url: "https://github.com/advisories/other",
  });
  assert.equal(accepted("vite", changed, packages, now), false);
});
test("blocks expired exceptions, unknown dependencies and cycles", () => {
  assert.equal(
    accepted("vite", findings, packages, Date.parse("2026-12-18")),
    false,
  );
  assert.equal(accepted("unknown", findings, packages, now), false);
  const changed = structuredClone(findings);
  changed.picomatch.via = ["vite"];
  assert.equal(accepted("vite", changed, packages, now), false);
});
