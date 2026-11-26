import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
export function upload(
  directory,
  platform,
  environment = process.env,
  execute = spawnSync,
) {
  const project =
    environment[
      platform === "web"
        ? "CRASH_ARTIFACTS_SENTRY_PROJECT_WEB"
        : "CRASH_ARTIFACTS_SENTRY_PROJECT_MOBILE"
    ];
  const token = environment.CRASH_ARTIFACTS_SENTRY_AUTH_TOKEN,
    org = environment.CRASH_ARTIFACTS_SENTRY_ORG;
  if (!token && !org && !project) return;
  if (!token || !org || !project)
    throw new Error("Incomplete crash artifact upload configuration");
  const manifest = JSON.parse(
    fs.readFileSync(path.join(directory, "manifest.json"), "utf8"),
  );
  const release =
    platform === "web"
      ? `nyxid-web@${manifest.app_version}+${manifest.artifact_id}`
      : `nyxid-${platform}@${manifest.app_version}+${manifest.app_build}`;
  const binary = path.join(
    path.dirname(fileURLToPath(import.meta.url)),
    "node_modules/.bin/sentry-cli",
  );
  const env = {
    ...environment,
    SENTRY_AUTH_TOKEN: token,
    SENTRY_ORG: org,
    SENTRY_PROJECT: project,
  };
  // Child output can contain operator configuration; expose a fixed error only.
  const run = (args) => {
    const result = execute(binary, args, { env, stdio: "pipe" });
    if (result.status !== 0) throw new Error("Crash artifact upload failed");
  };
  run(["releases", "new", release]);
  run([
    "sourcemaps",
    "upload",
    directory,
    "--release",
    release,
    "--dist",
    manifest.app_build,
    "--url-prefix",
    "app:///",
    "--validate",
    "--strict",
  ]);
  if (platform === "ios")
    run(["debug-files", "upload", path.join(directory, "dSYMs")]);
  run(["releases", "finalize", release]);
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [directory, platform] = process.argv.slice(2);
  if (!directory || !["web", "ios", "android"].includes(platform))
    throw new Error("Usage: upload.mjs <artifact-directory> web|ios|android");
  upload(path.resolve(directory), platform);
}
