// The CI workflow that builds the native apps (.github/workflows/native.yml): when it runs, on which
// runners, and what it builds, checks, and uploads.

import { existsSync, readFileSync } from "node:fs";
import { describe, expect, test } from "@rstest/core";
import { parse } from "yaml";

type Step = {
  name?: string;
  if?: string;
  uses?: string;
  run?: string;
  with?: Record<string, unknown>;
};
type Job = { "runs-on": string; steps: Step[] };
type Workflow = {
  on: {
    push: { branches: string[]; tags: string[]; paths: string[] };
    pull_request: { paths: string[] };
    workflow_dispatch: unknown;
  };
  concurrency: { "cancel-in-progress": string };
  permissions: Record<string, string>;
  jobs: Record<string, Job>;
};

const read = (file: string) => readFileSync(file, "utf8");

const workflow = parse(read("../../.github/workflows/native.yml")) as Workflow;
const linux = workflow.jobs["native-linux"];
const android = workflow.jobs["native-android"];

/** A GitHub Actions expression as the workflow writes it: `${{ body }}`. */
const expression = (body: string) => `\${{ ${body} }}`;

/** Every shell command a job runs, in order. */
function scripts(job: Job): string {
  return job.steps.map((step) => step.run ?? "").join("\n");
}

function uploads(job: Job): Step[] {
  return job.steps.filter((step) => step.uses?.startsWith("actions/upload-artifact@"));
}

/** The recipe of a Make target. */
function recipe(target: string): string {
  const match = new RegExp(`^${target}:\\n((?:\\t.*\\n)+)`, "m").exec(read("../../Makefile"));
  if (!match) throw new Error(`no ${target} target in the Makefile`);
  return match[1];
}

describe("native workflow triggers", () => {
  test("runs on version tags and on demand", () => {
    expect(workflow.on.push.tags).toEqual(["v*"]);
    expect(workflow.on).toHaveProperty("workflow_dispatch");
  });

  test("runs on pushes and pull requests that change the shell, the web app, or their build", () => {
    for (const paths of [workflow.on.push.paths, workflow.on.pull_request.paths]) {
      expect(paths).toEqual(
        expect.arrayContaining([
          "apps/native/**",
          "apps/web/**",
          ".github/workflows/native.yml",
          // The web app bundles the contracts.
          "packages/contracts/**",
          // The jobs run Make targets; the lockfile pins the Tauri CLI and its JS packages.
          "Makefile",
          "package.json",
          "pnpm-lock.yaml",
          "pnpm-workspace.yaml",
          ".nvmrc",
          "rust-toolchain.toml",
        ])
      );
    }
    expect(workflow.on.pull_request.paths).toEqual(workflow.on.push.paths);
  });

  test("every path the filter names exists, so a rename cannot silently skip the builds", () => {
    for (const path of workflow.on.push.paths) {
      expect(existsSync(`../../${path.replace(/\/\*\*$/, "")}`)).toBe(true);
    }
  });

  test("uploads no bundle built from a fork's pull request", () => {
    const steps = [...uploads(linux), ...uploads(android)];
    expect(steps).toHaveLength(3);
    for (const step of steps) {
      expect(step.if).toBe(
        "github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository"
      );
    }
  });

  test("never cancels a tag's build for a newer push", () => {
    expect(workflow.concurrency["cancel-in-progress"]).toBe(
      expression("!startsWith(github.ref, 'refs/tags/')")
    );
  });

  test("only reads the repository and pins every action to a commit", () => {
    expect(workflow.permissions).toEqual({ contents: "read" });
    for (const job of Object.values(workflow.jobs)) {
      for (const step of job.steps.filter((entry) => entry.uses)) {
        expect(step.uses).toMatch(/^[\w.-]+\/[\w.-]+@[0-9a-f]{40}$/);
      }
    }
  });
});

describe("native-linux", () => {
  test("builds on a pinned Ubuntu 24.04, whose glibc 2.39 is the floor of the bundles", () => {
    expect(linux["runs-on"]).toBe("ubuntu-24.04");
    expect(scripts(linux)).toContain('test "$(printf \'%s\\n\' "$glibc" 2.39 | sort -V');
  });

  test("lints and tests the shell and builds the .deb and the AppImage", () => {
    const commands = scripts(linux);
    expect(commands).toContain("make native-lint");
    expect(commands).toContain("make native-test");
    expect(recipe("native-test")).toContain("cargo test --manifest-path $(NATIVE_CRATE)");
    expect(commands).toContain("pnpm --filter @copilot/native tauri build --bundles deb,appimage");
  });

  test("checks the .deb's WebKitGTK dependency and the AppImage's type 2 magic", () => {
    const commands = scripts(linux);
    expect(commands).toContain('[[ "$depends" == *libwebkit2gtk-4.1-0* ]]');
    // `file` calls the static-pie runtime a "pie executable", so the check must not require "LSB executable".
    expect(commands).toContain('[[ "$kind" == "ELF 64-bit LSB "*", x86-64,"* ]]');
    expect(commands).not.toContain('"ELF 64-bit LSB executable');
    expect(commands).toContain('od -An -tx1 -j 8 -N 3 "$appimage" | tr -d \' \\n\')" = 414902');
  });

  test("uploads each bundle as an artifact of its own", () => {
    expect(uploads(linux).map((step) => step.with?.path)).toEqual([
      expression("steps.bundles.outputs.deb"),
      expression("steps.bundles.outputs.appimage"),
    ]);
    for (const step of uploads(linux)) expect(step.with).toMatchObject({ archive: false });
  });
});

describe("native-android", () => {
  test("builds on the standard runner with Java 21 and the preinstalled NDK 29", () => {
    expect(android["runs-on"]).toBe("ubuntu-latest");
    const java = android.steps.find((step) => step.uses?.startsWith("actions/setup-java@"));
    expect(java?.with).toMatchObject({ distribution: "temurin", "java-version": "21" });
    expect(scripts(android)).toContain("-name '29.*'");
    const rust = android.steps.find((step) => step.uses?.startsWith("dtolnay/rust-toolchain@"));
    expect(rust?.with).toMatchObject({ targets: "aarch64-linux-android" });
  });

  test("builds a release APK for arm64 only, the way make android-build does", () => {
    expect(scripts(android)).toContain("make android-build");
    expect(recipe("android-build")).toContain("tauri android build --apk --target aarch64");
    expect(recipe("android-build")).not.toContain("--debug");
  });

  test("checks that the APK is debug-signed, arm64-only, and named com.host39.copilot", () => {
    const commands = scripts(android);
    expect(commands).toContain('apksigner" verify --print-certs');
    expect(commands).toContain("grep -q 'CN=Android Debug'");
    expect(commands).toContain(
      'test "$(cut -d/ -f2 "$RUNNER_TEMP/libs.txt" | sort -u)" = arm64-v8a'
    );
    expect(commands).toContain("package: name='com.host39.copilot' ");
  });

  test("checks that the APK asks for the microphone and not for the camera", () => {
    const commands = scripts(android);
    for (const permission of ["RECORD_AUDIO", "MODIFY_AUDIO_SETTINGS"]) {
      expect(commands).toContain(
        `grep -qx "uses-permission: name='android.permission.${permission}'"`
      );
    }
    // `set -e` ignores a failed `! grep`, so the camera check needs an explicit exit.
    expect(commands).toContain(
      'if grep -q "android.permission.CAMERA" "$RUNNER_TEMP/badging.txt"; then'
    );
    expect(commands).not.toContain("! grep");
  });

  test("checks that the APK turns off backup and device transfer", () => {
    const commands = scripts(android);
    expect(commands).toContain('aapt2" dump xmltree --file AndroidManifest.xml "$apk"');
    expect(commands).toContain("grep -qE ':allowBackup\\(0x[0-9a-f]+\\)=false'");
    expect(commands).toContain("grep -qE ':dataExtractionRules\\(0x[0-9a-f]+\\)=@0x[0-9a-f]+'");
  });

  test("uploads the APK as an artifact", () => {
    expect(uploads(android).map((step) => step.with)).toEqual([
      expect.objectContaining({ path: expression("steps.apk.outputs.apk"), archive: false }),
    ]);
  });
});
