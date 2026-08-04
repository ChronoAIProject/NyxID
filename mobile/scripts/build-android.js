#!/usr/bin/env node
/**
 * Build a release Android .aab for the given APP_ENV profile.
 *
 * Flow:
 *   1. expo prebuild --platform android --clean  (android/ is gitignored)
 *   2. patch-android-build-gradle.js (force androidx.core 1.15.0)
 *   3. ./gradlew bundleRelease  — signing via -Pandroid.injected.signing.* flags
 *      that read from env (no need to patch build.gradle)
 *
 * Required env (in .env.{dev,prod} or .env.local):
 *   ANDROID_KEYSTORE_PATH        (default: ./credentials/release.keystore)
 *   ANDROID_KEYSTORE_PASSWORD
 *   ANDROID_KEY_ALIAS
 *   ANDROID_KEY_PASSWORD
 *
 * Generate a keystore once (forks each create their own):
 *   keytool -genkeypair -v -storetype PKCS12 \
 *     -keystore mobile/credentials/release.keystore \
 *     -alias nyxid -keyalg RSA -keysize 2048 -validity 10000
 *
 * !!! Save the keystore + passwords somewhere safe. Losing them locks
 *     you out of updating this app on Google Play permanently.
 *
 * Usage:  APP_ENV=prod node scripts/build-android.js
 * Output: prints the absolute path of the produced .aab on the last line.
 */
const { execSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const { loadEnv, resolveProfile, MOBILE_ROOT } = require("./lib/load-env");

const APP_ENV = process.env.APP_ENV === "dev" ? "dev" : "prod";
const env = loadEnv();
const r = resolveProfile(APP_ENV, env);

const KEYSTORE_PATH = env.ANDROID_KEYSTORE_PATH || "./credentials/release.keystore";
const KEYSTORE_PASSWORD = env.ANDROID_KEYSTORE_PASSWORD;
const KEY_ALIAS = env.ANDROID_KEY_ALIAS;
const KEY_PASSWORD = env.ANDROID_KEY_PASSWORD;

const missing = [];
if (!KEYSTORE_PASSWORD) missing.push("ANDROID_KEYSTORE_PASSWORD");
if (!KEY_ALIAS) missing.push("ANDROID_KEY_ALIAS");
if (!KEY_PASSWORD) missing.push("ANDROID_KEY_PASSWORD");
if (missing.length > 0) {
  console.error(
    `[build-android] Missing required env: ${missing.join(", ")}\n` +
      "Add them to mobile/.env.prod (or .env.local). See mobile/.env.example for layout.",
  );
  process.exit(1);
}

const keystoreAbs = path.isAbsolute(KEYSTORE_PATH)
  ? KEYSTORE_PATH
  : path.join(MOBILE_ROOT, KEYSTORE_PATH);

if (!fs.existsSync(keystoreAbs)) {
  console.error(
    `[build-android] Keystore not found at: ${keystoreAbs}\n` +
      "Generate one with:\n" +
      `  keytool -genkeypair -v -storetype PKCS12 -keystore ${KEYSTORE_PATH} \\\n` +
      "    -alias nyxid -keyalg RSA -keysize 2048 -validity 10000\n",
  );
  process.exit(1);
}

function run(cmd, opts = {}) {
  console.log(`\n$ ${cmd}\n`);
  execSync(cmd, { stdio: "inherit", cwd: MOBILE_ROOT, ...opts });
}

console.log(`[build-android] APP_ENV=${APP_ENV}`);
console.log(`[build-android] Android package: ${r.androidPackage}`);
console.log(`[build-android] versionCode:     ${r.androidVersionCode}`);
console.log(`[build-android] Keystore:        ${keystoreAbs}`);

console.log("\n[build-android] Step 1/3: expo prebuild --platform android --clean");
process.env.APP_ENV = APP_ENV;
run("npx expo prebuild --platform android --clean --no-install");

console.log("\n[build-android] Step 2/3: patch-android-build-gradle.js");
process.env.EAS_BUILD_PLATFORM = "android";
run(`node ${path.join("scripts", "patch-android-build-gradle.js")}`);

console.log("\n[build-android] Step 3/3: gradlew bundleRelease");
const gradleArgs = [
  ":app:bundleRelease",
  `-Pandroid.injected.signing.store.file=${keystoreAbs}`,
  `-Pandroid.injected.signing.store.password=${KEYSTORE_PASSWORD}`,
  `-Pandroid.injected.signing.key.alias=${KEY_ALIAS}`,
  `-Pandroid.injected.signing.key.password=${KEY_PASSWORD}`,
];
run(`./gradlew ${gradleArgs.join(" ")}`, {
  cwd: path.join(MOBILE_ROOT, "android"),
});

const aabPath = path.join(
  MOBILE_ROOT,
  "android",
  "app",
  "build",
  "outputs",
  "bundle",
  "release",
  "app-release.aab",
);
if (!fs.existsSync(aabPath)) {
  console.error("[build-android] No .aab produced at", aabPath);
  process.exit(1);
}
console.log(`\n[build-android] ✓ produced: ${aabPath}`);
console.log(`\nNext: pnpm submit:android   (uploads this .aab to Play Internal testing)`);
process.stdout.write(`\n${aabPath}\n`);
