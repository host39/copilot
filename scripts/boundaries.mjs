#!/usr/bin/env node
/**
 * Checks crate dependency boundaries declared in boundaries.config.mjs against
 * `cargo metadata`. Exits non-zero when an "error" rule is violated.
 *
 * Usage: node scripts/boundaries.mjs   (or `make boundaries`)
 */

import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const { default: config } = await import(
  pathToFileURL(resolve(ROOT, "boundaries.config.mjs")).href
);

const metadata = JSON.parse(
  execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
    cwd: ROOT,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  })
);

/** Direct normal and build dependencies per workspace crate. */
const dependencies = new Map(
  metadata.packages.map((pkg) => [
    pkg.name,
    new Set(pkg.dependencies.filter((dep) => dep.kind !== "dev").map((dep) => dep.name)),
  ])
);

let errors = 0;
let warnings = 0;

for (const rule of config.forbidden) {
  const deps = dependencies.get(rule.from);
  if (!deps) {
    console.error(`boundaries.rule.unknown-crate ${rule.name}: no workspace crate "${rule.from}"`);
    errors++;
    continue;
  }

  for (const target of rule.to) {
    if (!deps.has(target)) continue;
    const label = rule.severity === "error" ? "error" : "warn";
    console.error(`${label} ${rule.name}: ${rule.from} -> ${target}\n  ${rule.comment}`);
    if (rule.severity === "error") errors++;
    else warnings++;
  }
}

const checked = config.forbidden.length;
if (errors > 0) {
  console.error(
    `\nx ${errors} boundary violation(s), ${warnings} warning(s) across ${checked} rules`
  );
  process.exit(1);
}
console.log(`✔ no boundary violations (${checked} rules, ${dependencies.size} crates)`);
