#!/usr/bin/env node
/**
 * Upload an .ipa to App Store Connect / TestFlight via `xcrun altool`.
 *
 * Required env (account-wide; lives in .env.prod):
 *   ASC_API_KEY_ID         10-char Key ID
 *   ASC_API_KEY_ISSUER_ID  UUID
 *   APPLE_ID               your developer email (altool requires it for some checks)
 *
 * Required file:
 *   mobile/credentials/asc-api-key.p8
 *
 * altool looks for the .p8 in a few standard locations and the filename must be
 * `AuthKey_<KEY_ID>.p8`. This script copies our `asc-api-key.p8` into
 * `~/.appstoreconnect/private_keys/AuthKey_<KEY_ID>.p8` on each run so the
 * actual storage stays at our gitignored path.
 *
 * Usage:
 *   pnpm submit:ios            # auto-picks the most recent .ipa in ios/build/
 *   pnpm submit:ios <path>     # uploads the specified .ipa
 */
const { execSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const os = require("os");

const { loadEnv, MOBILE_ROOT } = require("./lib/load-env");

const env = loadEnv();

const missing = [];
if (!env.ASC_API_KEY_ID) missing.push("ASC_API_KEY_ID");
if (!env.ASC_API_KEY_ISSUER_ID) missing.push("ASC_API_KEY_ISSUER_ID");
if (!env.APPLE_ID) missing.push("APPLE_ID");
if (missing.length > 0) {
  console.error(`[submit-ios] Missing required env: ${missing.join(", ")}`);
  process.exit(1);
}

const p8Source = path.join(MOBILE_ROOT, "credentials", "asc-api-key.p8");
if (!fs.existsSync(p8Source)) {
  console.error(
    `[submit-ios] ASC API key not found at: ${p8Source}\n` +
      "Download the .p8 from ASC → Users and Access → Integrations → App Store Connect API\n" +
      "(remember: Apple only lets you download it once)."
  );
  process.exit(1);
}

let ipaPath = process.argv[2];
if (!ipaPath) {
  const buildDir = path.join(MOBILE_ROOT, "ios", "build");
  if (!fs.existsSync(buildDir)) {
    console.error("[submit-ios] No .ipa specified and ios/build/ does not exist. Run pnpm build:ios first.");
    process.exit(1);
  }
  const ipas = fs.readdirSync(buildDir).filter((f) => f.endsWith(".ipa"));
  if (ipas.length === 0) {
    console.error("[submit-ios] No .ipa found in ios/build/. Run pnpm build:ios first.");
    process.exit(1);
  }
  const stats = ipas.map((f) => ({ f, mtime: fs.statSync(path.join(buildDir, f)).mtimeMs }));
  stats.sort((a, b) => b.mtime - a.mtime);
  ipaPath = path.join(buildDir, stats[0].f);
  console.log(`[submit-ios] Auto-picked most recent .ipa: ${ipaPath}`);
}

if (!fs.existsSync(ipaPath)) {
  console.error(`[submit-ios] .ipa not found: ${ipaPath}`);
  process.exit(1);
}

// altool wants AuthKey_<KEY_ID>.p8 in ~/.appstoreconnect/private_keys/
const ascDir = path.join(os.homedir(), ".appstoreconnect", "private_keys");
const expectedKeyPath = path.join(ascDir, `AuthKey_${env.ASC_API_KEY_ID}.p8`);
fs.mkdirSync(ascDir, { recursive: true });
fs.copyFileSync(p8Source, expectedKeyPath);
console.log(`[submit-ios] Staged .p8 at ${expectedKeyPath}`);

const cmd = [
  "xcrun",
  "altool",
  "--upload-app",
  "--type", "ios",
  "--file", `"${ipaPath}"`,
  "--apiKey", env.ASC_API_KEY_ID,
  "--apiIssuer", env.ASC_API_KEY_ISSUER_ID,
].join(" ");

console.log(`\n[submit-ios] Uploading ${path.basename(ipaPath)} to App Store Connect...`);
console.log(`$ ${cmd}\n`);

try {
  execSync(cmd, { stdio: "inherit", cwd: MOBILE_ROOT });
} catch (e) {
  console.error("[submit-ios] altool exited non-zero. Common causes:");
  console.error("  - Build number ≤ last accepted (bump *_IOS_BUILD_NUMBER and rebuild)");
  console.error("  - Bundle ID mismatch with the ASC app");
  console.error("  - .p8 / Issuer ID / Key ID mismatch");
  process.exit(e.status || 1);
}

console.log(`\n[submit-ios] ✓ uploaded — check App Store Connect → TestFlight in a few minutes.`);
