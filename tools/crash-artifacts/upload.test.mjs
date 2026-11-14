import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { upload } from "./upload.mjs";
import { readBuildSecret } from "./build-web.mjs";

test("BuildKit secret parsing never exposes malformed secret text in errors or process output", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "nyx-build-secret-"));
  const filename = path.join(directory, "configuration.json");
  const token = "SECRET_SENTINEL_BEARER_BUILD_TOKEN";
  const configuration = {
    CRASH_ARTIFACTS_SENTRY_AUTH_TOKEN: token,
    CRASH_ARTIFACTS_SENTRY_ORG: "example",
    CRASH_ARTIFACTS_SENTRY_PROJECT_WEB: "web",
  };
  try {
    assert.deepEqual(readBuildSecret(filename), {});
    fs.writeFileSync(filename, "");
    assert.deepEqual(readBuildSecret(filename), {});
    fs.writeFileSync(filename, JSON.stringify(configuration));
    assert.deepEqual(readBuildSecret(filename), configuration);
    for (const text of [
      `{${token}`,
      JSON.stringify({ ...configuration, extra: token }),
      JSON.stringify({ CRASH_ARTIFACTS_SENTRY_AUTH_TOKEN: token }),
      JSON.stringify([token]),
      "null",
    ]) {
      fs.writeFileSync(filename, text);
      assert.throws(() => readBuildSecret(filename), {
        message: "Invalid crash artifact build configuration",
      });
      const result = spawnSync(
        process.execPath,
        [fileURLToPath(new URL("./build-web.mjs", import.meta.url)), filename],
        { encoding: "utf8" },
      );
      assert.equal(result.status, 1);
      assert.equal(result.stdout, "");
      assert.equal(
        result.stderr,
        "Invalid crash artifact build configuration\n",
      );
      assert.ok(!JSON.stringify(result).includes(token));
    }
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

test("artifact upload is optional and partial configuration fails closed", () => {
  const execute = () =>
    assert.fail("Unconfigured upload must not invoke a process");
  upload("/absent", "web", {}, execute);
  assert.throws(
    () =>
      upload(
        "/absent",
        "web",
        { CRASH_ARTIFACTS_SENTRY_ORG: "example" },
        execute,
      ),
    /Incomplete crash artifact upload configuration/,
  );
});

test("web, iOS and Android upload commands match the pinned CLI and keep tokens out of arguments", () => {
  const directory = fs.mkdtempSync(
    path.join(os.tmpdir(), "nyx-artifact-proof-"),
  );
  const manifest = {
    app_version: "1.2.3",
    app_build: "456",
    artifact_id: "2e6e467d-54b7-4839-bfb1-c3e563368b37",
  };
  const token = "SECRET_SENTINEL";
  const environment = {
    CRASH_ARTIFACTS_SENTRY_AUTH_TOKEN: token,
    CRASH_ARTIFACTS_SENTRY_ORG: "example",
    CRASH_ARTIFACTS_SENTRY_PROJECT_WEB: "web",
    CRASH_ARTIFACTS_SENTRY_PROJECT_MOBILE: "mobile",
  };
  fs.writeFileSync(
    path.join(directory, "manifest.json"),
    JSON.stringify(manifest),
  );
  try {
    for (const platform of ["web", "ios", "android"]) {
      const calls = [];
      upload(directory, platform, environment, (binary, args, options) => {
        calls.push(args);
        assert.equal(options.env.SENTRY_AUTH_TOKEN, token);
        assert.equal(
          options.env.SENTRY_PROJECT,
          platform === "web" ? "web" : "mobile",
        );
        assert.equal(options.stdio, "pipe");
        assert.ok(!JSON.stringify(args).includes(token));
        // --help validates clap's production command/option parser without network I/O.
        const result = spawnSync(binary, [...args, "--help"], {
          encoding: "utf8",
        });
        assert.equal(result.status, 0, result.stderr);
        return { status: 0 };
      });
      const release =
        platform === "web"
          ? `nyxid-web@1.2.3+${manifest.artifact_id}`
          : `nyxid-${platform}@1.2.3+456`;
      assert.deepEqual(calls[0], ["releases", "new", release]);
      assert.deepEqual(calls[1], [
        "sourcemaps",
        "upload",
        directory,
        "--release",
        release,
        "--dist",
        "456",
        "--url-prefix",
        "app:///",
        "--validate",
        "--strict",
      ]);
      assert.equal(calls.length, platform === "ios" ? 4 : 3);
      if (platform === "ios")
        assert.deepEqual(calls[2], [
          "debug-files",
          "upload",
          path.join(directory, "dSYMs"),
        ]);
      assert.deepEqual(calls.at(-1), ["releases", "finalize", release]);
    }
    assert.throws(
      () =>
        upload(directory, "web", environment, () => ({
          status: 1,
          stderr: token,
        })),
      (error) =>
        error.message === "Crash artifact upload failed" &&
        !String(error).includes(token),
    );
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

test("web private maps are retained only for release intent and always removed from dist", async () => {
  const { collectCrashArtifacts } =
    await import("../../frontend/scripts/collect-crash-artifacts.mjs");
  for (const retain of [undefined, "1"]) {
    const directory = fs.mkdtempSync(
      path.join(os.tmpdir(), "nyx-map-retention-"),
    );
    const root = path.join(directory, "frontend");
    fs.mkdirSync(path.join(root, "dist/assets"), { recursive: true });
    fs.writeFileSync(
      path.join(root, "dist/crash-build.json"),
      JSON.stringify({ artifact_id: "2e6e467d-54b7-4839-bfb1-c3e563368b37" }),
    );
    fs.writeFileSync(path.join(root, "dist/assets/app.js"), "minified");
    fs.writeFileSync(
      path.join(root, "dist/assets/app.js.map"),
      "private symbols",
    );
    const result = await collectCrashArtifacts(
      root,
      retain ? { CRASH_ARTIFACTS_RETAIN: retain } : {},
    );
    assert.equal(
      fs.existsSync(path.join(root, "dist/assets/app.js.map")),
      false,
    );
    assert.equal(fs.existsSync(result.target), retain === "1");
    fs.rmSync(directory, { recursive: true, force: true });
  }
});
test("iOS and Android never upload with missing build secrets", () => {
  for (const platform of ["ios", "android"])
    for (const keys of [
      [],
      ["CRASH_ARTIFACTS_SENTRY_AUTH_TOKEN"],
      ["CRASH_ARTIFACTS_SENTRY_ORG"],
      ["CRASH_ARTIFACTS_SENTRY_PROJECT_MOBILE"],
      ["CRASH_ARTIFACTS_SENTRY_AUTH_TOKEN", "CRASH_ARTIFACTS_SENTRY_ORG"],
    ]) {
      const environment = Object.fromEntries(keys.map((k) => [k, "sentinel"]));
      let calls = 0;
      try {
        upload("/absent", platform, environment, () => {
          calls++;
        });
      } catch (error) {
        assert.match(
          error.message,
          /Incomplete crash artifact upload configuration/,
        );
      }
      assert.equal(calls, 0);
    }
});

test("failed public-map assertions also delete unrequested private copies", async () => {
  const { collectCrashArtifacts } =
    await import("../../frontend/scripts/collect-crash-artifacts.mjs");
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "nyx-map-failure-"));
  const root = path.join(directory, "frontend");
  const artifact = "2e6e467d-54b7-4839-bfb1-c3e563368b37";
  try {
    fs.mkdirSync(path.join(root, "dist/assets"), { recursive: true });
    fs.writeFileSync(
      path.join(root, "dist/crash-build.json"),
      JSON.stringify({ artifact_id: artifact }),
    );
    fs.writeFileSync(
      path.join(root, "dist/assets/app.js"),
      "//# sourceMappingURL=app.js.map",
    );
    fs.writeFileSync(
      path.join(root, "dist/assets/app.js.map"),
      "private symbols",
    );
    await assert.rejects(
      collectCrashArtifacts(root, {}),
      /Public diagnostic symbols/,
    );
    assert.equal(
      fs.existsSync(path.join(root, "dist/assets/app.js.map")),
      false,
    );
    assert.equal(
      fs.existsSync(path.join(directory, "crash-artifacts/web", artifact)),
      false,
    );
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

test("collector accepts a shared supplied UUID and rejects malformed identities", async () => {
  const { collectCrashArtifacts } = await import("../../frontend/scripts/collect-crash-artifacts.mjs");
  const { buildArtifactId } = await import("../../frontend/scripts/crash-build-identity.mjs");
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "nyx-shared-artifact-"));
  const root = path.join(directory, "frontend");
  fs.mkdirSync(path.join(root, "dist/assets"), { recursive: true });
  try {
    const id = buildArtifactId("2e6e467d-54b7-4839-bfb1-c3e563368b37");
    fs.writeFileSync(path.join(root, "dist/crash-build.json"), JSON.stringify({ artifact_id: id }));
    fs.writeFileSync(path.join(root, "dist/assets/app.js"), "closed");
    fs.writeFileSync(path.join(root, "dist/assets/app.js.map"), "{}");
    const result = await collectCrashArtifacts(root, {});
    assert.ok(result.target.endsWith(id));
    assert.equal(fs.existsSync(path.join(root, "dist/assets/app.js.map")), false);
    fs.writeFileSync(path.join(root, "dist/crash-build.json"), JSON.stringify({ artifact_id: "-".repeat(36) }));
    await assert.rejects(collectCrashArtifacts(root, {}), /Missing crash artifact identity/);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});
