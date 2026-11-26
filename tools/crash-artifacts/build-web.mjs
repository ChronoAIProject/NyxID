import fs from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const configurationError = "Invalid crash artifact build configuration";
const keys = [
  "CRASH_ARTIFACTS_SENTRY_AUTH_TOKEN",
  "CRASH_ARTIFACTS_SENTRY_ORG",
  "CRASH_ARTIFACTS_SENTRY_PROJECT_WEB",
];

export function readBuildSecret(filename) {
  try {
    if (!fs.existsSync(filename)) return {};
    const raw = fs.readFileSync(filename, "utf8");
    if (!raw.trim()) return {};
    const value = JSON.parse(raw);
    if (
      value === null ||
      typeof value !== "object" ||
      Array.isArray(value) ||
      Object.keys(value).some((key) => !keys.includes(key)) ||
      (Object.keys(value).length > 0 &&
        keys.some((key) => typeof value[key] !== "string" || !value[key]))
    )
      throw new Error(configurationError);
    return value;
  } catch {
    // JSON parser errors may include the secret's source text.
    throw new Error(configurationError);
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const value = readBuildSecret(
      process.argv[2] ?? "/run/secrets/crash_artifacts_sentry",
    );
    const result = spawnSync("npm", ["run", "build"], {
      stdio: "inherit",
      env: { ...process.env, ...value },
    });
    process.exitCode = result.status ?? 1;
  } catch {
    process.stderr.write(`${configurationError}\n`);
    process.exitCode = 1;
  }
}
