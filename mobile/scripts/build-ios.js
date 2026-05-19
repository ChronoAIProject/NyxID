#!/usr/bin/env node
/**
 * Build a release iOS .ipa for the given APP_ENV profile.
 *
 * Flow:
 *   1. expo prebuild --platform ios (regenerates ios/ from app.config.ts;
 *      no --clean so manual CocoaPods / AppDelegate customizations stay)
 *   2. pod install
 *   3. xcodebuild archive  — automatic signing using APPLE_TEAM_ID from env
 *   4. xcodebuild -exportArchive — produces the .ipa
 *
 * Usage:  APP_ENV=prod node scripts/build-ios.js
 * Output: prints the absolute path of the produced .ipa on the last line.
 */
const { execSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const os = require("os");

const { loadEnv, resolveProfile, MOBILE_ROOT } = require("./lib/load-env");

const APP_ENV = process.env.APP_ENV === "dev" ? "dev" : "prod";
const env = loadEnv();
const r = resolveProfile(APP_ENV, env);

if (!env.APPLE_TEAM_ID) {
  console.error(
    "[build-ios] APPLE_TEAM_ID is not set. Add it to mobile/.env.prod\n" +
      "(Apple → Developer → Membership → Team ID, 10 chars uppercase+digits).",
  );
  process.exit(1);
}

const APPLE_TEAM_ID = env.APPLE_TEAM_ID;
const SCHEME = "NyxIDMobile";
const WORKSPACE = path.join(MOBILE_ROOT, "ios", `${SCHEME}.xcworkspace`);

function run(cmd, opts = {}) {
  console.log(`\n$ ${cmd}\n`);
  execSync(cmd, { stdio: "inherit", cwd: MOBILE_ROOT, ...opts });
}

console.log(`[build-ios] APP_ENV=${APP_ENV}`);
console.log(`[build-ios] iOS bundle ID:      ${r.iosBundleId}`);
console.log(`[build-ios] iOS build number:   ${r.iosBuildNumber}`);
console.log(`[build-ios] APPLE_TEAM_ID:      ${APPLE_TEAM_ID}`);

console.log("\n[build-ios] Step 1/4: expo prebuild --platform ios");
process.env.APP_ENV = APP_ENV;
run("npx expo prebuild --platform ios --no-install");

console.log("\n[build-ios] Step 2/4: pod install");
run("pod install", { cwd: path.join(MOBILE_ROOT, "ios") });

const buildDir = path.join(MOBILE_ROOT, "ios", "build");
const archivePath = path.join(buildDir, `${SCHEME}.xcarchive`);
const ipaOutputDir = buildDir;

// Do NOT wipe ios/build/ here — pod install just wrote React Native
// codegen output to ios/build/generated/ that xcodebuild needs. Only
// remove a stale .xcarchive from a previous run.
if (fs.existsSync(archivePath)) {
  fs.rmSync(archivePath, { recursive: true, force: true });
}
fs.mkdirSync(buildDir, { recursive: true });
// Clean any previous .ipa export so we don't pick up a stale one later.
for (const f of fs.readdirSync(buildDir)) {
  if (f.endsWith(".ipa")) {
    fs.rmSync(path.join(buildDir, f));
  }
}

console.log("\n[build-ios] Step 3/4: xcodebuild archive");
run(
  [
    "xcodebuild",
    "-workspace", `"${WORKSPACE}"`,
    "-scheme", SCHEME,
    "-configuration", "Release",
    "-destination", '"generic/platform=iOS"',
    "-archivePath", `"${archivePath}"`,
    "-allowProvisioningUpdates",
    `DEVELOPMENT_TEAM=${APPLE_TEAM_ID}`,
    "CODE_SIGN_STYLE=Automatic",
    "archive",
  ].join(" "),
);

const exportOptionsPath = path.join(buildDir, "ExportOptions.plist");
const exportOptions = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key>
  <string>app-store-connect</string>
  <key>teamID</key>
  <string>${APPLE_TEAM_ID}</string>
  <key>uploadBitcode</key>
  <false/>
  <key>uploadSymbols</key>
  <true/>
  <key>signingStyle</key>
  <string>automatic</string>
  <key>stripSwiftSymbols</key>
  <true/>
  <key>destination</key>
  <string>export</string>
</dict>
</plist>
`;
fs.writeFileSync(exportOptionsPath, exportOptions);

console.log("\n[build-ios] Step 4/4: xcodebuild -exportArchive");
run(
  [
    "xcodebuild",
    "-exportArchive",
    "-archivePath", `"${archivePath}"`,
    "-exportPath", `"${ipaOutputDir}"`,
    "-exportOptionsPlist", `"${exportOptionsPath}"`,
    "-allowProvisioningUpdates",
  ].join(" "),
);

const ipas = fs.readdirSync(ipaOutputDir).filter((f) => f.endsWith(".ipa"));
if (ipas.length === 0) {
  console.error("[build-ios] No .ipa produced in", ipaOutputDir);
  process.exit(1);
}
const ipaPath = path.join(ipaOutputDir, ipas[0]);
console.log(`\n[build-ios] ✓ produced: ${ipaPath}`);
console.log(`\nNext: pnpm submit:ios   (uploads this .ipa to TestFlight)`);
// Last line is just the path so submit-ios.js can pipe it
process.stdout.write(`\n${ipaPath}\n`);
