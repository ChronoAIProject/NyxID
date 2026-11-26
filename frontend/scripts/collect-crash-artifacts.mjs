import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { isCrashArtifactId } from "./crash-build-identity.mjs";
export async function collectCrashArtifacts(root, environment = process.env) {
  const dist = path.join(root, "dist");
  const identity = JSON.parse(
    fs.readFileSync(path.join(dist, "crash-build.json"), "utf8"),
  );
  if (!isCrashArtifactId(identity.artifact_id))
    throw new Error("Missing crash artifact identity");
  const target = path.resolve(
    root,
    "../crash-artifacts/web",
    identity.artifact_id,
  );
  fs.mkdirSync(target, { recursive: true });
  const maps = [];
  function walk(directory) {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const filename = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        walk(filename);
        continue;
      }
      const relative = path.relative(dist, filename);
      if (entry.name.endsWith(".map")) {
        const destination = path.join(target, relative);
        fs.mkdirSync(path.dirname(destination), { recursive: true });
        fs.renameSync(filename, destination);
        maps.push(relative);
        const asset = filename.slice(0, -4);
        if (fs.existsSync(asset))
          fs.copyFileSync(asset, path.join(target, relative.slice(0, -4)));
      }
    }
  }
  const uploadConfigured = [
    "CRASH_ARTIFACTS_SENTRY_AUTH_TOKEN",
    "CRASH_ARTIFACTS_SENTRY_ORG",
    "CRASH_ARTIFACTS_SENTRY_PROJECT_WEB",
  ].every((key) => Boolean(environment[key]));
  try {
    walk(dist);
    function assertPrivate(directory) {
      for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
        const filename = path.join(directory, entry.name);
        if (entry.isDirectory()) assertPrivate(filename);
        else if (
          entry.name.endsWith(".map") ||
          fs.readFileSync(filename).includes(Buffer.from("sourceMappingURL"))
        )
          throw new Error(
            `Public diagnostic symbols: ${path.relative(dist, filename)}`,
          );
      }
    }
    assertPrivate(dist);
    if (!maps.length) throw new Error("Hidden source maps were not produced");
    fs.writeFileSync(
      path.join(target, "manifest.json"),
      JSON.stringify({ ...identity, maps }, null, 2) + "\n",
    );
    const { upload } = await import("../../tools/crash-artifacts/upload.mjs");
    upload(target, "web", environment);
  } finally {
    if (!uploadConfigured && environment.CRASH_ARTIFACTS_RETAIN !== "1")
      fs.rmSync(target, { recursive: true, force: true });
  }
  return { target, maps };
}
if (process.argv[1] === fileURLToPath(import.meta.url))
  await collectCrashArtifacts(
    path.resolve(path.dirname(fileURLToPath(import.meta.url)), ".."),
  );
