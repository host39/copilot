#!/usr/bin/env node
/**
 * Bootstrap a new project from this template.
 *
 * Usage:
 *   node scripts/create-project.mjs <target-dir> --name <project-name> [--scope <@scope>] [--title <Display Name>] [--skip-git-init]
 *
 * Example:
 *   node scripts/create-project.mjs ../my-app --name my-app --title "My App"
 *
 * What it does:
 *   1. Copies the template to <target-dir> (excluding target, node_modules, .git, build output).
 *   2. Rewrites token names across file contents:
 *        @copilot/   -> @<scope>/            (npm workspace scope)
 *        Copilot     -> <title>              (display name; defaults to the project name)
 *        copilot-    -> <project-name>-      (Cargo package names)
 *        copilot_    -> <project_name>_      (Rust crate identifiers)
 *        copilot     -> <project_name>       (database name, root package name)
 *   3. Optionally initializes a fresh git repository.
 */

import { execFileSync } from "node:child_process";
import { cp, readdir, readFile, stat, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const TEMPLATE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const EXCLUDED_DIRS = new Set(["node_modules", ".git", "target", "dist", "out", ".run"]);

// File extensions / names whose contents we rewrite tokens in.
const TEXT_FILE_PATTERN =
  /\.(m?[jt]sx?|json|md|mdc|ya?ml|sql|css|html|bru|env|sample|example|rs|toml|lock|sh|py)$/i;
const TEXT_FILE_NAMES = new Set([
  ".gitignore",
  ".nvmrc",
  ".editorconfig",
  ".markdownlintignore",
  "Makefile",
  "Dockerfile",
  "pre-commit",
]);

function parseArgs(argv) {
  const positionals = [];
  const flags = {};
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg.startsWith("--")) {
      const key = arg.slice(2);
      if (key === "skip-git-init") {
        flags[key] = true;
      } else {
        flags[key] = argv[++i];
      }
    } else {
      positionals.push(arg);
    }
  }
  return { positionals, flags };
}

function toSnakeCase(name) {
  return name
    .replace(/[-\s]+/g, "_")
    .replace(/[^a-zA-Z0-9_]/g, "")
    .toLowerCase();
}

function toSlug(name) {
  return name
    .replace(/[_\s]+/g, "-")
    .replace(/[^a-zA-Z0-9-]/g, "")
    .toLowerCase();
}

function normalizeScope(scope, slug) {
  const raw = (scope ?? slug).replace(/^@/, "");
  return `@${raw}`;
}

async function collectFiles(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    if (entry.isDirectory()) {
      if (EXCLUDED_DIRS.has(entry.name)) continue;
      files.push(...(await collectFiles(join(dir, entry.name))));
    } else {
      files.push(join(dir, entry.name));
    }
  }
  return files;
}

function isTextFile(path) {
  const name = path.split("/").pop() ?? "";
  return TEXT_FILE_NAMES.has(name) || TEXT_FILE_PATTERN.test(name);
}

/** Applies `rewrite` to every text file under `root`; returns how many files changed. */
async function rewriteTokens(root, rewrite) {
  let rewritten = 0;
  for (const file of await collectFiles(root)) {
    if (!isTextFile(file)) continue;
    const info = await stat(file);
    if (info.size > 2_000_000) continue;

    const original = await readFile(file, "utf8");
    const updated = rewrite(original);
    if (updated !== original) {
      await writeFile(file, updated);
      rewritten++;
    }
  }
  return rewritten;
}

async function main() {
  const { positionals, flags } = parseArgs(process.argv.slice(2));
  const target = positionals[0];
  const projectName = flags.name;

  if (!target || !projectName) {
    console.error(
      "Usage: node scripts/create-project.mjs <target-dir> --name <project-name> [--scope <@scope>] [--title <Display Name>] [--skip-git-init]"
    );
    process.exit(1);
  }

  const targetPath = resolve(process.cwd(), target);
  const slug = toSlug(projectName);
  const snake = toSnakeCase(projectName);
  const scope = normalizeScope(flags.scope, slug);
  const title = flags.title ?? projectName;

  if (!/^[a-z][a-z0-9-]*$/.test(slug)) {
    console.error(`Project name must start with a letter: ${projectName}`);
    process.exit(1);
  }

  // Guard: don't clobber a non-empty directory.
  try {
    const existing = await readdir(targetPath);
    if (existing.length > 0) {
      console.error(`Target directory is not empty: ${targetPath}`);
      process.exit(1);
    }
  } catch {
    // Directory does not exist yet; cp will create it.
  }

  console.log(`Creating project "${projectName}" at ${targetPath}`);
  console.log(`  scope:        ${scope}`);
  console.log(`  crate prefix: ${slug}- (${snake}_)`);
  console.log(`  db name:      ${snake}`);
  console.log(`  title:        ${title}`);

  await cp(TEMPLATE_ROOT, targetPath, {
    recursive: true,
    filter: (src) => {
      const rel = src.slice(TEMPLATE_ROOT.length + 1);
      if (!rel) return true;
      const segments = rel.split("/");
      const name = segments.at(-1) ?? "";
      // Local env files hold machine-specific values and secrets; `make setup` recreates them.
      if (name === ".env" || /^\.env.*\.local$/.test(name)) return false;
      return !segments.some((segment) => EXCLUDED_DIRS.has(segment));
    },
  });

  const rewritten = await rewriteTokens(targetPath, (text) =>
    text
      .replaceAll("@copilot/", `${scope}/`)
      .replaceAll("Copilot", title)
      .replaceAll("copilot-", `${slug}-`)
      .replaceAll("copilot_", `${snake}_`)
      .replaceAll("copilot", snake)
  );
  console.log(`Rewrote tokens in ${rewritten} file(s).`);

  if (!flags["skip-git-init"]) {
    try {
      execFileSync("git", ["init"], { cwd: targetPath, stdio: "ignore" });
      console.log("Initialized a new git repository.");
    } catch {
      console.warn("Could not initialize git repository (git not available?).");
    }
  }

  console.log("\nDone. Next steps:");
  console.log(`  cd ${target}`);
  console.log("  make install");
  console.log("  make setup");
  console.log("  make dev");
}

main().catch((error) => {
  console.error("Failed to create project:", error);
  process.exit(1);
});
